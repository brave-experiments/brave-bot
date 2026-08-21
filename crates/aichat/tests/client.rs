//! Integration tests against a mock chat completions server.
//!
//! These check what unit tests cannot: that the request actually carries the signing
//! headers the server verifies, that it reaches the right path, and that the reply
//! arrives labelled untrusted.

use bua_aichat::AichatClient;
use bua_aichat::protocol::{ChatRequest, Message};
use bua_config::Config;
use bua_core::capability::{Capability, CapabilitySet};
use bua_core::event::RecordingSink;
use bua_core::label::Label;
use bua_core::policy::{Policy, ReleasePlan, Routing};
use bua_net::Egress;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

/// What the server received, so tests can assert on the request rather than only the
/// response.
struct Captured {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Captured {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Serve one canned response, returning the base URL and a channel carrying what was
/// received.
fn serve(response_body: &str) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (sender, receiver) = mpsc::channel();
    let response_body = response_body.to_string();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));

        let mut request_line = String::new();
        reader.read_line(&mut request_line).expect("request line");

        let mut headers = Vec::new();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                let name = name.trim().to_string();
                let value = value.trim().to_string();
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.parse().unwrap_or(0);
                }
                headers.push((name, value));
            }
        }

        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).expect("body");

        let _ = sender.send(Captured {
            request_line: request_line.trim().to_string(),
            headers,
            body: String::from_utf8_lossy(&body).to_string(),
        });

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    });

    (format!("http://127.0.0.1:{port}"), receiver)
}

fn config_for(endpoint: &str) -> Config {
    Config::from_lookup(|key| match key {
        "SERVICES_KEY_AICHAT" => Some("test-signing-key".into()),
        "BRAVE_SERVICES_KEY_ID" => Some("test-key-id".into()),
        "BRAVE_AI_CHAT_ENDPOINT" => Some(endpoint.to_string()),
        _ => None,
    })
    .expect("config")
}

fn routing() -> Routing {
    let mut r = Routing::new();
    r.insert_trusted("task", "say hello");
    r
}

const REPLY: &str = r#"{"model":"served-model","choices":[{"message":{"role":"assistant","content":"hello from the model"}}]}"#;

#[test]
fn a_completion_round_trips() {
    let (endpoint, received) = serve(REPLY);
    let config = config_for(&endpoint);
    let egress = Egress::new();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy");

    let client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    let completion = client
        .complete(&mut policy, &request)
        .expect("completion succeeds");

    assert_eq!(completion.model, "served-model");
    // Model output is untrusted, whatever it says.
    assert_eq!(completion.content.label(), Label::untrusted_public());

    let captured = received.recv().expect("request captured");
    assert!(
        captured
            .request_line
            .starts_with("POST /v1/chat/completions"),
        "wrong target: {}",
        captured.request_line
    );
}

/// The server rejects anything without a matching signature, so the headers must be
/// present and in the exact form it verifies.
#[test]
fn the_request_carries_the_signing_headers() {
    let (endpoint, received) = serve(REPLY);
    let config = config_for(&endpoint);
    let egress = Egress::new();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy");

    let client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    client.complete(&mut policy, &request).expect("completion");

    let captured = received.recv().expect("request captured");

    let digest = captured.header("digest").expect("digest header");
    assert!(digest.starts_with("SHA-256="), "malformed digest: {digest}");
    // The digest must be over the body actually sent.
    assert_eq!(digest, bua_signing::digest_header(captured.body.as_bytes()));

    let authorization = captured
        .header("authorization")
        .expect("authorization header");
    assert!(authorization.starts_with("Signature keyId=\"test-key-id\""));
    assert!(authorization.contains("algorithm=\"hs2019\""));
    // The server rejects any signed-header set other than exactly "digest".
    assert!(authorization.contains("headers=\"digest\""));

    assert_eq!(
        captured.header("content-type"),
        Some("application/json"),
        "the server expects json"
    );

    // The signing key must never be transmitted.
    for (name, value) in &captured.headers {
        assert!(
            !value.contains("test-signing-key"),
            "the signing key leaked in {name}"
        );
    }
}

#[test]
fn the_request_body_matches_the_protocol() {
    let (endpoint, received) = serve(REPLY);
    let config = config_for(&endpoint);
    let egress = Egress::new();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy");

    let client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new(
        "automatic",
        vec![Message::system("be brief"), Message::user("hi")],
    );
    client.complete(&mut policy, &request).expect("completion");

    let captured = received.recv().expect("request captured");
    let body: serde_json::Value = serde_json::from_str(&captured.body).expect("json body");

    assert_eq!(body["model"], "automatic");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["content"], "hi");
}

/// Without the fetch capability the request must not leave, and the policy records it.
#[test]
fn a_completion_without_the_capability_is_refused() {
    let (endpoint, _received) = serve(REPLY);
    let config = config_for(&endpoint);
    let egress = Egress::new();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::none(),
        &mut sink,
    )
    .expect("policy");

    let client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    let error = client
        .complete(&mut policy, &request)
        .expect_err("must be refused");

    assert!(error.to_string().contains("web_fetch"), "got: {error}");
    assert!(!policy.finish());
}

#[test]
fn a_response_without_content_is_an_error() {
    let (endpoint, _received) = serve(r#"{"model":"m","choices":[]}"#);
    let config = config_for(&endpoint);
    let egress = Egress::new();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy");

    let client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    let error = client
        .complete(&mut policy, &request)
        .expect_err("no content is an error");
    assert!(error.to_string().contains("no message content"));
}
