//! Integration tests against a mock chat completions server.
//!
//! These check what unit tests cannot: that the request actually carries the signing
//! headers the server verifies, that it reaches the right path, and that the reply
//! arrives labelled untrusted.

use bravebot_aichat::AichatClient;
use bravebot_aichat::protocol::{ChatRequest, Message};
use bravebot_config::Config;
use bravebot_core::capability::{Capability, CapabilitySet};
use bravebot_core::event::RecordingSink;
use bravebot_core::label::Label;
use bravebot_core::policy::{Policy, ReleasePlan, Routing};
use bravebot_net::Egress;
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

/// Serve an SSE stream, writing each frame separately and flushing between them.
///
/// Written frame by frame rather than as one body, because that is the condition the decoder has
/// to survive: a payload can be split across reads, and one arriving whole in a single read would
/// not exercise the buffering at all.
fn serve_stream(frames: Vec<String>) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (sender, receiver) = mpsc::channel();

    // Every connection, not just the first. A reply that stops early is sent again, so a server
    // that answered once and went away would turn a retried request into a refused one, and the
    // test would be asserting on the wrong failure.
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
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

            // No content-length: the stream ends when the connection closes, as a real one does.
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            );
            let _ = stream.flush();

            for frame in &frames {
                let _ = stream.write_all(frame.as_bytes());
                let _ = stream.flush();
            }
        }
    });

    (format!("http://127.0.0.1:{port}"), receiver)
}

/// What the server does with one connection.
enum Attempt {
    /// Read the request and hang up without answering, the way a connection that died looks.
    Dropped,
    /// Answer with a status and nothing else.
    Status(u16),
    /// Answer properly, with these SSE frames.
    Frames(Vec<String>),
}

/// Serve one behaviour per connection, in order, recording what each request carried.
///
/// A retry is a second connection, so a server that behaves differently on each is the only way
/// to tell one apart from a client that gave up.
fn serve_attempts(attempts: Vec<Attempt>) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        for attempt in attempts {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));

            let mut request_line = String::new();
            if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                break;
            }

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
            let _ = reader.read_exact(&mut body);

            let _ = sender.send(Captured {
                request_line: request_line.trim().to_string(),
                headers,
                body: String::from_utf8_lossy(&body).to_string(),
            });

            match attempt {
                Attempt::Dropped => drop(stream),
                Attempt::Status(status) => {
                    let _ = stream.write_all(
                        format!("HTTP/1.1 {status} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .as_bytes(),
                    );
                    let _ = stream.flush();
                }
                Attempt::Frames(frames) => {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                    );
                    for frame in frames {
                        let _ = stream.write_all(frame.as_bytes());
                        let _ = stream.flush();
                    }
                }
            }
        }
    });

    (format!("http://127.0.0.1:{port}"), receiver)
}

/// One `data:` frame carrying a chunk.
fn frame(payload: &str) -> String {
    format!("data: {payload}\n\n")
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

    let mut client = AichatClient::new(&config, &egress);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    client.complete(&mut policy, &request).expect("completion");

    let captured = received.recv().expect("request captured");

    let digest = captured.header("digest").expect("digest header");
    assert!(digest.starts_with("SHA-256="), "malformed digest: {digest}");
    // The digest must be over the body actually sent.
    assert_eq!(
        digest,
        bravebot_signing::digest_header(captured.body.as_bytes())
    );

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

    let mut client = AichatClient::new(&config, &egress);
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

    let mut client = AichatClient::new(&config, &egress);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    let error = client
        .complete(&mut policy, &request)
        .expect_err("no content is an error");
    assert!(error.to_string().contains("no message content"));
}

/// A streamed reply must arrive as the same completion a buffered one would have produced, and
/// the count must climb on the way rather than appearing only at the end.
#[test]
fn a_streamed_completion_arrives_in_pieces() {
    let (endpoint, received) = serve_stream(vec![
        frame(r#"{"model":"served-model","choices":[{"delta":{"role":"assistant"}}]}"#),
        frame(r#"{"choices":[{"delta":{"content":"hello"}}]}"#),
        frame(r#"{"choices":[{"delta":{"content":" from"}}]}"#),
        frame(r#"{"choices":[{"delta":{"content":" the model"}}]}"#),
        frame(
            r#"{"choices":[{"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":3}}"#,
        ),
        frame("[DONE]"),
    ]);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);

    // The witness a caller with a screen mints. Without one the words in each report cannot be
    // read at all, which is the arrangement: a caller with nowhere to draw them never asks.
    let shown = policy.authorise_display_release("the reply as the model writes it");

    let mut seen = Vec::new();
    let mut written = String::new();
    let completion = client
        .complete_streaming(&mut policy, &request, |progress| {
            written.push_str(progress.written.declassify(&shown));
            seen.push((progress.output_tokens, progress.counted_by_server));
        })
        .expect("streamed completion succeeds");

    // Each report carries what arrived since the last one, so the pieces put back together are
    // the reply. Sending the whole of it every time would cost the square of a long answer.
    assert_eq!(written, "hello from the model");

    assert_eq!(completion.model, "served-model");
    // Streamed or not, model output is untrusted.
    assert_eq!(completion.content.label(), Label::untrusted_public());
    assert_eq!(completion.usage.completion_tokens, 3);

    // The count rose while the reply arrived, which is the point of streaming it.
    let counts: Vec<u64> = seen.iter().map(|(count, _)| *count).collect();
    assert!(
        counts.windows(2).all(|w| w[1] >= w[0]),
        "the count went backwards: {counts:?}"
    );
    assert!(
        counts.iter().any(|c| *c > 0),
        "the count never moved: {counts:?}"
    );
    // And it ends on the server's figure, not the estimate.
    assert_eq!(seen.last().expect("progress was reported").0, 3);
    assert!(seen.last().expect("reported").1);

    let captured = received.recv().expect("request captured");
    assert!(
        captured.body.contains("\"stream\":true"),
        "the request did not ask to stream: {}",
        captured.body
    );
    assert!(
        captured.body.contains("\"include_usage\":true"),
        "the request did not ask for usage: {}",
        captured.body
    );
}

/// A server that hangs up mid-reply leaves the caller at the end of the bytes, exactly as one
/// that finished does. Told apart only by what the server said, a cut-off reply came back as a
/// whole one: the tool call the model was part way through writing vanished, and the turn ended
/// on what read as a considered answer.
#[test]
fn a_stream_that_stops_before_the_server_says_it_is_finished_is_not_a_reply() {
    let (endpoint, _received) = serve_stream(vec![
        frame(r#"{"model":"m","choices":[{"delta":{"content":"Let me look at"}}]}"#),
        frame(r#"{"choices":[{"delta":{"content":" the render code:"}}]}"#),
        // and then nothing: no finish reason, no end-of-stream payload.
    ]);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    let failed = client
        .complete_streaming(&mut policy, &request, |_| {})
        .expect_err("a reply that stopped early is not an answer");

    assert!(
        failed
            .to_string()
            .contains("before the server said it was finished"),
        "got: {failed}"
    );
}

/// The end-of-stream payload and a finish reason are two ways of saying the same thing, and a
/// server may send either. Requiring the one this backend happens to send would reject every
/// reply from a server that sends only the other.
#[test]
fn either_way_of_saying_the_reply_is_over_is_accepted() {
    for terminator in [
        vec![frame(r#"{"choices":[{"finish_reason":"stop"}]}"#)],
        vec![frame("[DONE]")],
    ] {
        let mut frames = vec![frame(
            r#"{"model":"m","choices":[{"delta":{"content":"done"}}]}"#,
        )];
        frames.extend(terminator);

        let (endpoint, _received) = serve_stream(frames);
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

        let mut client = AichatClient::new(&config, &egress);
        let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
        client
            .complete_streaming(&mut policy, &request, |_| {})
            .expect("a reply the server said was over is an answer");
    }
}

/// Tool calls arrive fragmented, and a streamed round has to reassemble them into something
/// dispatchable or tool use would break the moment streaming was turned on.
#[test]
fn a_streamed_tool_call_is_reassembled() {
    let (endpoint, _received) = serve_stream(vec![
        frame(
            r#"{"model":"m","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":""}}]}}]}"#,
        ),
        frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\""}}]}}]}"#,
        ),
        frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"a.rs\"}"}}]}}]}"#,
        ),
        frame(
            r#"{"choices":[{"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":8,"completion_tokens":5}}"#,
        ),
        frame("[DONE]"),
    ]);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("read a.rs")]);
    let completion = client
        .complete_streaming(&mut policy, &request, |_| {})
        .expect("a tool-calling stream succeeds");

    assert_eq!(completion.calls.len(), 1);
    assert_eq!(completion.calls[0].function.name, "read_file");
    assert_eq!(
        completion.calls[0].arguments().expect("parses")["path"],
        "a.rs"
    );
}

/// The gate runs before any body exists, so a streamed request with no capability is refused
/// exactly as a buffered one is. Streaming must not be a way around the check.
#[test]
fn a_streamed_request_without_the_capability_is_refused() {
    let (endpoint, _received) = serve_stream(vec![frame("[DONE]")]);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    let error = client
        .complete_streaming(&mut policy, &request, |_| {})
        .expect_err("must be refused");

    assert!(error.to_string().contains("web_fetch"), "got: {error}");
    assert!(!policy.finish());
}

/// A stream that carried nothing usable is an error rather than an empty reply presented as an
/// answer.
#[test]
fn a_stream_with_no_content_is_an_error() {
    let (endpoint, _received) = serve_stream(vec![
        frame(r#"{"model":"m","choices":[{"delta":{"role":"assistant"}}]}"#),
        frame("[DONE]"),
    ]);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    let error = client
        .complete_streaming(&mut policy, &request, |_| {})
        .expect_err("no content is an error");
    assert!(error.to_string().contains("no message content"));
}

/// A frame the server sends that is not a chunk must not discard a reply that is otherwise
/// arriving: keepalives and comments are normal.
#[test]
fn unparseable_frames_do_not_lose_the_reply() {
    let (endpoint, _received) = serve_stream(vec![
        ": keepalive\n\n".to_string(),
        frame("not json at all"),
        frame(r#"{"model":"m","choices":[{"delta":{"content":"still here"}}]}"#),
        frame("[DONE]"),
    ]);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    let completion = client
        .complete_streaming(&mut policy, &request, |_| {})
        .expect("a stream with noise in it still succeeds");

    let proof = policy.authorise_display_release("test reads the reply");
    assert_eq!(completion.content.declassify(&proof), "still here");
}

/// A stub subscription handing out one credential, so routing can be tested without a keychain.
struct StubSubscription {
    remaining: usize,
}

impl bravebot_aichat::Subscription for StubSubscription {
    fn next_credential(&mut self) -> Result<bravebot_aichat::SubscriptionCredential, String> {
        if self.remaining == 0 {
            return Err("the imported credentials are used up".to_string());
        }
        self.remaining -= 1;
        Ok(bravebot_aichat::SubscriptionCredential {
            cookie_name: "__Secure-sku#brave-leo-premium".to_string(),
            cookie_value: "presented-credential".to_string(),
        })
    }
}

fn premium_config(endpoint: &str, premium: &str) -> Config {
    Config::from_lookup(|key| match key {
        "SERVICES_KEY_AICHAT" => Some("test-signing-key".into()),
        "BRAVE_SERVICES_KEY_ID" => Some("test-key-id".into()),
        "BRAVE_AI_CHAT_ENDPOINT" => Some(endpoint.to_string()),
        "BRAVE_AI_CHAT_PREMIUM_ENDPOINT" => Some(premium.to_string()),
        _ => None,
    })
    .expect("config")
}

/// With a subscription, the request must go to the premium host and carry the credential as the
/// cookie the backend reads. This is what the whole import exists to produce.
#[test]
fn a_subscribed_request_goes_to_the_premium_host_with_the_credential() {
    let (premium_endpoint, received) = serve(REPLY);
    // The free host is a port nothing is listening on, so reaching it would fail rather than
    // quietly pass.
    let config = premium_config("http://127.0.0.1:1", &premium_endpoint);
    let egress = Egress::new();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy");

    let mut subscription = StubSubscription { remaining: 1 };
    let mut client = AichatClient::new(&config, &egress).with_subscription(&mut subscription);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    client
        .complete(&mut policy, &request)
        .expect("completion succeeds");

    let captured = received.recv().expect("request captured");
    assert_eq!(
        captured.header("cookie"),
        Some("__Secure-sku#brave-leo-premium=presented-credential")
    );
}

/// Once the batch is spent the request must fail rather than quietly going out on the free tier.
/// A downgrade nobody was told about is indistinguishable from the service getting worse, and it
/// would also spend a premium-tier allowance the user thought they had paid past.
#[test]
fn an_exhausted_subscription_fails_rather_than_downgrading() {
    // Both hosts point at a listener, so a fallback would succeed and this would pass wrongly.
    let (free_endpoint, _received) = serve(REPLY);
    let config = premium_config(&free_endpoint, &free_endpoint);
    let egress = Egress::new();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy");

    let mut subscription = StubSubscription { remaining: 0 };
    let mut client = AichatClient::new(&config, &egress).with_subscription(&mut subscription);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);

    let err = client
        .complete(&mut policy, &request)
        .expect_err("an unusable subscription must fail the request");
    let shown = err.to_string();
    assert!(shown.contains("subscription"), "unclear error: {shown}");
    assert!(
        shown.contains("import-leo-creds"),
        "no remedy offered: {shown}"
    );
}

/// A build with no premium host must stay on the free tier even when credentials exist, rather
/// than attaching one to a request bound for the free endpoint.
#[test]
fn without_a_premium_host_no_credential_is_attached() {
    let (free_endpoint, received) = serve(REPLY);
    let config = config_for(&free_endpoint);
    let egress = Egress::new();
    let mut sink = RecordingSink::new();
    let mut policy = Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        &mut sink,
    )
    .expect("policy");

    let mut subscription = StubSubscription { remaining: 5 };
    let mut client = AichatClient::new(&config, &egress).with_subscription(&mut subscription);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    client
        .complete(&mut policy, &request)
        .expect("completion succeeds");

    let captured = received.recv().expect("request captured");
    assert_eq!(captured.header("cookie"), None);
    // The credential must not have been spent either, since it was never usable here.
    assert_eq!(subscription.remaining, 5);
}

/// The failure that started this: a machine sleeps, the connection it had is gone, and the reply
/// that would have arrived never does. Nothing about the request has changed, so it is sent
/// again rather than handed back to the user as an error they have to act on.
#[test]
fn a_request_that_died_in_transit_is_sent_again() {
    let (endpoint, received) = serve_attempts(vec![
        Attempt::Dropped,
        Attempt::Frames(vec![
            frame(r#"{"model":"served-model","choices":[{"delta":{"content":"hello"}}]}"#),
            frame("[DONE]"),
        ]),
    ]);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    let mut attempts = Vec::new();
    let completion = client
        .complete_streaming(&mut policy, &request, |progress| {
            attempts.push(progress.attempt);
        })
        .expect("the second attempt succeeds");

    assert_eq!(completion.model, "served-model");

    let first = received.recv().expect("a first request");
    let second = received.recv().expect("a second request");
    assert_eq!(
        first.body, second.body,
        "the retry must be the same request, not a different one"
    );

    assert!(
        attempts.contains(&2),
        "the caller was not told the reply had started over: {attempts:?}"
    );
}

/// A request that was answered was not a transport failure, and asking again would be asking a
/// server that already said no.
#[test]
fn a_request_the_server_refused_is_not_sent_again() {
    let (endpoint, received) = serve_attempts(vec![
        Attempt::Status(400),
        Attempt::Frames(vec![frame("[DONE]")]),
    ]);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    client
        .complete_streaming(&mut policy, &request, |_| {})
        .expect_err("a refused request stays refused");

    received.recv().expect("a first request");
    assert!(
        received
            .recv_timeout(std::time::Duration::from_millis(500))
            .is_err(),
        "the request was sent a second time"
    );
}

/// Every attempt is a request in its own right, so the gate has to see each one. Retrying past a
/// refusal would be a way to send something the policy had already stopped.
#[test]
fn a_retry_goes_through_the_gate_again() {
    let (endpoint, _received) = serve_attempts(vec![
        Attempt::Dropped,
        Attempt::Frames(vec![
            frame(r#"{"model":"served-model","choices":[{"delta":{"content":"hi"}}]}"#),
            frame("[DONE]"),
        ]),
    ]);
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

    let mut client = AichatClient::new(&config, &egress);
    let request = ChatRequest::new("automatic", vec![Message::user("hi")]);
    client
        .complete_streaming(&mut policy, &request, |_| {})
        .expect("the second attempt succeeds");

    let checks = sink
        .events()
        .iter()
        .filter(|event| {
            matches!(
                event,
                bravebot_core::event::Event::GatePassed {
                    gate: "network",
                    ..
                }
            )
        })
        .count();
    assert_eq!(checks, 2, "each attempt must be checked on its own");
}

/// The listing is a plain GET on the free host. It carries no signature and no credential: the
/// endpoint requires neither, and spending a subscription credential to read a public list would
/// be spending one for nothing.
#[test]
fn the_model_listing_is_fetched_from_the_models_path() {
    let listed = r#"[{"key":"claude-3-sonnet","display_name":"Claude Sonnet",
        "capabilities":["chat","tools"],"options":{"access":"premium"}}]"#;
    let (endpoint, received) = serve(listed);
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

    let models =
        bravebot_aichat::models::list(&mut policy, &config, &egress).expect("the list arrives");

    assert_eq!(models[0].key, "automatic");
    assert_eq!(models[1].key, "claude-3-sonnet");

    let captured = received.recv().expect("request captured");
    assert!(
        captured.request_line.starts_with("GET /v1/models"),
        "wrong target: {}",
        captured.request_line
    );
    assert!(
        captured.header("authorization").is_none(),
        "the listing was signed"
    );
    assert!(
        captured.header("cookie").is_none(),
        "a credential was spent on the listing"
    );
}

/// Every request leaves through the egress gate, so one made without the capability must be
/// refused rather than quietly reaching the network by another route.
#[test]
fn the_model_listing_is_refused_without_the_network_capability() {
    let (endpoint, _received) = serve("[]");
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

    let refused = bravebot_aichat::models::list(&mut policy, &config, &egress);
    assert!(refused.is_err(), "the listing bypassed the gate");
}

/// A body that is not the shape the endpoint documents is an error, not an empty list: a picker
/// showing only "automatic" would look like a server with one model rather than a broken reply.
#[test]
fn a_listing_that_is_not_an_array_is_an_error() {
    let (endpoint, _received) = serve(r#"{"data":[]}"#);
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

    let refused = bravebot_aichat::models::list(&mut policy, &config, &egress);
    assert!(refused.is_err(), "an envelope was accepted as a list");
}
