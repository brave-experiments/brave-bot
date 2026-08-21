//! Integration tests against a real loopback HTTP server.
//!
//! The unit tests cover URL and cap logic in isolation; these check the behaviour that
//! only appears when bytes actually move: that the policy gate runs before the request,
//! that every redirect hop is revalidated, and that a response body arrives labelled.

use bua_core::capability::{Capability, CapabilitySet};
use bua_core::event::{Event, RecordingSink};
use bua_core::label::Label;
use bua_core::policy::{Policy, ReleasePlan, Routing};
use bua_net::{Egress, Request};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

/// A single-shot server that replies with a canned sequence, one response per
/// connection, then stops.
fn serve(responses: Vec<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();

    thread::spawn(move || {
        for response in responses {
            match listener.accept() {
                Ok((stream, _)) => handle(stream, &response),
                Err(_) => break,
            }
        }
    });

    format!("http://127.0.0.1:{port}")
}

fn handle(mut stream: TcpStream, response: &str) {
    // Read the request head so the client is not writing into a closed socket.
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        if line == "\r\n" || line == "\n" {
            break;
        }
        line.clear();
    }
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn ok_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn redirect_to(location: &str) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn routing() -> Routing {
    let mut r = Routing::new();
    r.insert_trusted("task", "fetch a page");
    r
}

#[test]
fn a_successful_fetch_returns_a_labelled_body() {
    let base = serve(vec![ok_response("hello from the server")]);
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy begins");

    let egress = Egress::new();
    let response = egress
        .fetch(&mut policy, Request::get(&base), Label::untrusted_public())
        .expect("fetch succeeds");

    assert_eq!(response.status, 200);
    assert_eq!(response.body.label(), Label::untrusted_public());
    assert!(!response.truncated);
}

/// Without the fetch capability nothing should leave the process, and the failure must
/// come from the gate rather than from a connection error.
#[test]
fn a_fetch_without_the_capability_is_refused() {
    let base = serve(vec![ok_response("should never be read")]);
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::none(),
        &mut sink,
    )
    .expect("policy begins");

    let egress = Egress::new();
    let error = egress
        .fetch(&mut policy, Request::get(&base), Label::untrusted_public())
        .expect_err("must be refused");

    assert!(error.to_string().contains("web_fetch"));
    assert!(!policy.finish(), "the refusal must be recorded");
}

/// The property the manual redirect loop exists for: each hop is checked, so the gate
/// sees the redirect target and not only the original URL.
#[test]
fn every_redirect_hop_is_revalidated() {
    // Both responses come from the same server, since a path-absolute Location
    // resolves against the host it was served from.
    let first = serve(vec![
        redirect_to("/second"),
        ok_response("final destination"),
    ]);

    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy begins");

    let egress = Egress::new();
    let response = egress
        .fetch(
            &mut policy,
            Request::get(format!("{first}/first")),
            Label::untrusted_public(),
        )
        .expect("redirect is followed");
    assert_eq!(response.status, 200);

    // Two network gate events: the original URL and the redirect target.
    let checked: Vec<&String> = sink
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::GatePassed {
                gate: "network",
                detail,
            } => Some(detail),
            _ => None,
        })
        .collect();

    assert_eq!(checked.len(), 2, "expected one check per hop: {checked:?}");
    assert!(checked[0].contains("/first"));
    assert!(
        checked[1].contains("/second"),
        "the redirect target was not revalidated: {checked:?}"
    );
}

/// A redirect chain that never terminates must stop rather than loop forever.
#[test]
fn a_redirect_loop_is_bounded() {
    // More redirects than the cap allows, all pointing at the same server.
    let responses: Vec<String> = (0..10).map(|_| redirect_to("/again")).collect();
    let base = serve(responses);

    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy begins");

    let egress = Egress::new();
    let error = egress
        .fetch(
            &mut policy,
            Request::get(format!("{base}/start")),
            Label::untrusted_public(),
        )
        .expect_err("must give up");

    assert!(
        error.to_string().contains("too many redirects"),
        "unexpected error: {error}"
    );
}

#[test]
fn a_non_success_status_is_an_error() {
    let base = serve(vec![
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
    ]);

    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy begins");

    let egress = Egress::new();
    let error = egress
        .fetch(&mut policy, Request::get(&base), Label::untrusted_public())
        .expect_err("404 is an error");
    assert!(error.to_string().contains("404"), "unexpected: {error}");
}

/// A non-http scheme must be rejected before any connection is attempted, so the
/// network path cannot be used to read local files.
#[test]
fn non_http_schemes_never_reach_the_network() {
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy begins");

    let egress = Egress::new();
    let error = egress
        .fetch(
            &mut policy,
            Request::get("file:///etc/passwd"),
            Label::untrusted_public(),
        )
        .expect_err("must be refused");

    assert!(error.to_string().contains("only http and https"));
    // Refused before the gate, so no network check was recorded at all.
    assert!(
        !sink.events().iter().any(|e| matches!(
            e,
            Event::GatePassed {
                gate: "network",
                ..
            }
        )),
        "a non-http url should not reach the network gate"
    );
}
