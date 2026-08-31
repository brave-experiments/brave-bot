//! Integration tests against a real loopback HTTP server.
//!
//! The unit tests cover URL and cap logic in isolation; these check the behaviour that
//! only appears when bytes actually move: that the policy gate runs before the request,
//! that every redirect hop is revalidated, and that a response body arrives labelled.

use bravebot_core::capability::{Capability, CapabilitySet};
use bravebot_core::event::{Event, RecordingSink};
use bravebot_core::label::Label;
use bravebot_core::policy::{Policy, ReleasePlan, Routing};
use bravebot_net::{Egress, EgressError, Request, Timeouts};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

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

/// A server that sends its headers at once and then writes the body a piece at a time.
///
/// This is what a model streaming a long answer looks like on the wire, and what a single
/// end-to-end timeout could not tell apart from a stalled connection.
fn serve_trickled(pieces: Vec<&'static str>, gap: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if line == "\r\n" || line == "\n" {
                break;
            }
            line.clear();
        }

        let length: usize = pieces.iter().map(|p| p.len()).sum();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.flush();

        for piece in pieces {
            thread::sleep(gap);
            let _ = stream.write_all(piece.as_bytes());
            let _ = stream.flush();
        }
    });

    format!("http://127.0.0.1:{port}")
}

/// A server that accepts the connection and then says nothing at all.
fn serve_silence() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();

    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        // Held open, unanswered, past anything the test waits for: dropping it would itself be
        // an answer of a kind, and the point is a connection that says nothing at all.
        thread::sleep(Duration::from_secs(60));
        drop(stream);
    });

    format!("http://127.0.0.1:{port}")
}

/// The bug a closed laptop lid exposed. A reply that is still being written takes longer than
/// any one phase of the request allows, and cutting it off for that is cutting off a working
/// request for working slowly.
#[test]
fn a_reply_still_arriving_is_not_cut_off_for_taking_longer_than_it_took_to_start() {
    let base = serve_trickled(
        vec!["one ", "two ", "three ", "four ", "five"],
        Duration::from_millis(120),
    );
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy begins");

    // The reply takes several times longer than the longest gap allowed within it, which is
    // precisely what a single end-to-end bound cannot express.
    let egress = Egress::with_timeouts(Timeouts {
        idle: Duration::from_millis(400),
        reply: Duration::from_secs(10),
        ..Timeouts::default()
    });

    let response = egress
        .fetch(&mut policy, Request::get(&base), Label::untrusted_public())
        .expect("a slowly written body still arrives");

    assert_eq!(response.status, 200);
    assert!(!response.truncated);
    let (body, _) = response.body.into_parts_for_decoding();
    assert_eq!(String::from_utf8_lossy(&body), "one two three four five");
}

/// The other half of the same property: a request that is getting nowhere still ends.
#[test]
fn a_reply_that_never_comes_gives_up() {
    let base = serve_silence();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy begins");

    let egress = Egress::with_timeouts(Timeouts {
        reply: Duration::from_millis(300),
        ..Timeouts::default()
    });

    let error = egress
        .fetch(&mut policy, Request::get(&base), Label::untrusted_public())
        .expect_err("a server that never answers is not waited on forever");

    assert!(
        matches!(error, EgressError::Transport { .. }),
        "expected a transport failure, got {error:?}"
    );
}

/// A server that starts a reply and then stops, without closing the connection.
///
/// The shape of a machine that went to sleep mid-request: the bytes stop, and nothing at either
/// end says the connection is over.
fn serve_stalled_body() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if line == "\r\n" || line == "\n" {
                break;
            }
            line.clear();
        }

        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 100\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(b"the beginning of an answer");
        let _ = stream.flush();
        thread::sleep(Duration::from_secs(60));
    });

    format!("http://127.0.0.1:{port}")
}

/// The gap bound is what makes a dead connection detectable at all, and it has to be measured
/// between pieces rather than from the start, or it would be the end-to-end bound again.
#[test]
fn a_reply_that_stops_arriving_is_given_up_on() {
    let base = serve_stalled_body();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy begins");

    let egress = Egress::with_timeouts(Timeouts {
        idle: Duration::from_millis(300),
        reply: Duration::from_secs(30),
        ..Timeouts::default()
    });

    let mut stream = egress
        .fetch_streaming(&mut policy, Request::get(&base), Label::untrusted_public())
        .expect("the reply starts arriving");

    let error = loop {
        match stream.next_chunk() {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the body should not have ended cleanly"),
            Err(error) => break error,
        }
    };

    assert!(
        matches!(error, EgressError::Transport { .. }),
        "expected a transport failure, got {error:?}"
    );
}

/// A buffered read has to tell the difference too. Silently handing back the part that arrived
/// turns a dead connection into whatever those bytes happen to parse as, which for a JSON reply
/// is a puzzling decoding error somewhere far from the cause.
#[test]
fn a_body_that_stops_partway_is_a_failure_rather_than_a_short_body() {
    let base = serve_stalled_body();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy begins");

    let egress = Egress::with_timeouts(Timeouts {
        idle: Duration::from_millis(300),
        reply: Duration::from_secs(30),
        ..Timeouts::default()
    });

    let error = egress
        .fetch(&mut policy, Request::get(&base), Label::untrusted_public())
        .expect_err("half a body is not a body");

    assert!(
        matches!(error, EgressError::Transport { .. }),
        "expected a transport failure, got {error:?}"
    );
}
