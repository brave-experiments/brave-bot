//! Client for an OpenAI-compatible backend.
//!
//! Targets `POST /v1/chat/completions`. Despite the `/v1` prefix that is the **v2**
//! API for Brave's own endpoint: the server infers the version from the path, so there is no
//! `/v2/` route to construct. `/v1/conversation` is the older, deprecated surface.
//!
//! Two services are reached through this one client, because they speak the same protocol. Brave's
//! endpoint signs its requests; see [`bravebot_signing`]. A configured gateway bearer-authenticates
//! and may carry pass-through options in the body. What differs between them is a URL, a header and
//! a body field, which is why [`AichatClient::prepare`] is the only place that branches on it: the
//! framing, the retries and the cancellation are the same code for both.
//!
//! All traffic goes through [`bravebot_net::Egress`] so the policy gate sees it, and the model
//! reported in the response is preserved because the server may substitute a different
//! one than was requested.

pub mod models;
pub mod protocol;

use bravebot_config::Config;
use bravebot_core::cancel::Cancel;
use bravebot_core::event::Sink;
use bravebot_core::label::Label;
use bravebot_core::policy::Policy;
use bravebot_core::value::Labelled;
use bravebot_net::{Egress, EgressError, Request};
use protocol::{ChatChunk, ChatRequest, ChatResponse, STREAM_DONE, SseDecoder, StreamAccumulator};
use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub enum ChatError {
    /// The request could not be serialised.
    Encode(String),
    /// The response was not the expected shape.
    Decode { detail: String },
    /// The request never left, or failed in transit.
    Egress(EgressError),
    /// A well-formed response carrying no usable content.
    NoContent,
    /// The stream stopped without the server saying the reply was over.
    ///
    /// Distinct from [`ChatError::NoContent`]: there may be a great deal of content, and the
    /// problem is that there was going to be more. Retried like a connection that died, because
    /// that is most often what it is.
    Incomplete,
    /// A subscription is configured but no credential could be presented.
    ///
    /// Fails the request rather than falling back: see [`AichatClient::route`].
    Subscription(String),
    /// The caller asked for the reply to stop arriving, part way through reading it.
    ///
    /// Never retried: it is the one error that says the answer is no longer wanted.
    Cancelled,
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(detail) => write!(f, "could not encode the request: {detail}"),
            Self::Decode { detail } => write!(f, "unexpected response: {detail}"),
            Self::Egress(e) => write!(f, "{e}"),
            Self::NoContent => f.write_str("the response contained no message content"),
            Self::Incomplete => {
                f.write_str("the reply stopped before the server said it was finished")
            }
            Self::Cancelled => f.write_str("the reply was stopped while it was arriving"),
            Self::Subscription(detail) => write!(
                f,
                "the Leo subscription could not be used: {detail}. Run `bravebot import-leo-creds` to \
                 refresh it, or unset the premium endpoint to use the free tier"
            ),
        }
    }
}

impl std::error::Error for ChatError {}

impl From<EgressError> for ChatError {
    fn from(value: EgressError) -> Self {
        Self::Egress(value)
    }
}

/// A completion, with the model the server actually used.
#[derive(Debug)]
pub struct Completion {
    /// The assistant's reply. Untrusted: it is model output, so it may carry anything
    /// an injected instruction put there.
    pub content: Labelled<String>,
    /// The model reported by the server, which may differ from the one requested:
    /// unrecognised names are reset to automatic, and some entries resolve randomly
    /// within a weighted ensemble.
    pub model: String,
    /// Tools the model asked to call. Empty when it answered directly.
    ///
    /// The arguments are model output and therefore untrusted; a caller must gate them
    /// before letting any of it direct an operation.
    pub calls: Vec<protocol::ToolCall>,
    /// What this round cost, as the server counted it.
    pub usage: protocol::Usage,
}

/// A source of subscription credentials, one per request.
///
/// A trait rather than a stored string because each credential is single-use: presenting one
/// spends it, so a request has to ask for its own rather than reuse a cached value. It is also
/// what keeps this crate independent of where credentials come from.
pub trait Subscription {
    /// The cookie value presenting the next credential.
    ///
    /// An error here fails the request. It deliberately does not fall back to the free tier: a
    /// configured subscription that silently stops being used looks like the model got worse for
    /// no reason, and the one thing worse than an error is an unexplained downgrade.
    fn next_credential(&mut self) -> Result<SubscriptionCredential, String>;
}

/// A credential ready to be attached to one request.
pub struct SubscriptionCredential {
    /// The cookie name the backend reads.
    pub cookie_name: String,
    /// The presented credential.
    pub cookie_value: String,
}

/// Redacting rather than derived: the value is a bearer credential, and the obvious debugging
/// reflex of printing a request would otherwise put a live one in a log.
impl fmt::Debug for SubscriptionCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SubscriptionCredential({}=<redacted>)", self.cookie_name)
    }
}

pub struct AichatClient<'a> {
    config: &'a Config,
    egress: &'a Egress,
    subscription: Option<&'a mut dyn Subscription>,
    cancel: Option<Cancel>,
    gateway: Option<Gateway<'a>>,
}

/// Where a request goes when a configured gateway serves the model, rather than Brave's endpoint.
///
/// Holds the resolved token rather than the means of resolving one, because a caller has already
/// had to find it to know whether the request can be sent at all.
struct Gateway<'a> {
    provider: &'a bravebot_config::provider::Provider,
    token: String,
}

impl<'a> AichatClient<'a> {
    pub fn new(config: &'a Config, egress: &'a Egress) -> Self {
        Self {
            config,
            egress,
            subscription: None,
            cancel: None,
            gateway: None,
        }
    }

    /// Send this request to a configured gateway, bearer-authenticated, instead of to Brave.
    ///
    /// The model has already decided this: a caller reaches for it because the name it holds is one
    /// the provider offers. Nothing here re-decides that, and a token is required rather than
    /// optional because an unauthenticated request to a gateway is one that fails at the far end for
    /// a reason nothing local could explain.
    pub fn for_gateway(
        mut self,
        provider: &'a bravebot_config::provider::Provider,
        token: impl Into<String>,
    ) -> Self {
        self.gateway = Some(Gateway {
            provider,
            token: token.into(),
        });
        self
    }

    /// Send requests on the premium tier, spending a credential on each.
    pub fn with_subscription(mut self, subscription: &'a mut dyn Subscription) -> Self {
        self.subscription = Some(subscription);
        self
    }

    /// Stop reading a streamed reply as soon as this says to.
    ///
    /// A stream only ever reads, so there is nothing part done to leave behind and no reason to
    /// go on reading a reply nobody is waiting for. Without it, a person who has stopped the turn
    /// watches the rest of the answer arrive first, which is the whole of what they asked to
    /// stop. A caller that offers no way to say so is never stopped.
    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Where this request goes, and any credential to attach.
    ///
    /// The premium host and the credential travel together: a credential belongs to the premium
    /// deployment, so a build with no premium host stays on the free tier rather than sending the
    /// credential somewhere it does not belong.
    ///
    /// With both a premium host and a subscription, this is premium or nothing. A credential that
    /// cannot be produced fails the request rather than quietly reverting to the free tier, because
    /// a downgrade nobody was told about is indistinguishable from the service getting worse.
    fn route(&mut self) -> Result<(String, Option<SubscriptionCredential>), ChatError> {
        let free = self.config.chat_completions_url();

        let Some(premium_url) = self.config.premium_chat_completions_url() else {
            return Ok((free, None));
        };

        match self.subscription.as_mut() {
            Some(source) => match source.next_credential() {
                Ok(credential) => Ok((premium_url, Some(credential))),
                Err(detail) => Err(ChatError::Subscription(detail)),
            },
            // Premium is configured but nothing has been imported, which is not an error: the free
            // tier is what an unsubscribed caller gets.
            None => Ok((free, None)),
        }
    }

    /// The request to send, addressed and authenticated for whichever service serves it.
    ///
    /// The one place either backend is branched on. Both speak the same protocol over the same
    /// egress path, so what a gateway changes is the host, the credential, and a body that may carry
    /// pass-through options; everything after this point is shared.
    ///
    /// The options are merged at the top level of the body and never interpreted. They come from the
    /// settings file, which is the person's own configuration surface, so they are trusted exactly as
    /// far as a variable they exported would be. A key the request already set wins, because a
    /// configuration must not rewrite the model or the messages a turn built.
    fn prepare(&mut self, request: &ChatRequest) -> Result<Request, ChatError> {
        let encode = |value: &serde_json::Value| {
            serde_json::to_vec(value).map_err(|e| ChatError::Encode(e.to_string()))
        };

        if let Some(gateway) = self.gateway.as_ref() {
            let mut body =
                serde_json::to_value(request).map_err(|e| ChatError::Encode(e.to_string()))?;
            if let (Some(object), Some(options)) = (
                body.as_object_mut(),
                gateway
                    .provider
                    .model(&request.model)
                    .and_then(|model| model.options.as_ref()),
            ) {
                for (key, value) in options {
                    object.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
            return Ok(
                Request::post(gateway.provider.chat_completions_url(), encode(&body)?)
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {}", gateway.token)),
            );
        }

        let body = serde_json::to_vec(request).map_err(|e| ChatError::Encode(e.to_string()))?;
        let headers =
            bravebot_signing::sign(self.config.signing_key.expose(), &self.config.key_id, &body);
        let (url, credential) = self.route()?;

        let mut http = Request::post(url, body)
            .header("content-type", "application/json")
            .header("digest", &headers.digest)
            .header("authorization", &headers.authorization);
        if let Some(credential) = credential {
            http = http.header(
                "cookie",
                format!("{}={}", credential.cookie_name, credential.cookie_value),
            );
        }
        Ok(http)
    }

    /// Send a chat completion request.
    ///
    /// The reply is labelled untrusted-public: it is remote content we do not control,
    /// but carries no confidentiality of ours.
    pub fn complete<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
    ) -> Result<Completion, ChatError> {
        let mut attempt = 1;
        loop {
            match self.complete_once(policy, request) {
                Err(error) if worth_another_attempt(attempt, &error) => {
                    std::thread::sleep(backoff(attempt));
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    /// One attempt at [`AichatClient::complete`].
    fn complete_once<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
    ) -> Result<Completion, ChatError> {
        let http = self.prepare(request)?;

        let response = self.egress.fetch(policy, http, Label::untrusted_public())?;

        // Decoding the transport envelope needs the raw bytes, so the label is taken
        // out explicitly and reapplied to the extracted text below. The assistant's
        // reply therefore stays untrusted; only the envelope is treated as protocol.
        let (bytes, label) = response.body.into_parts_for_decoding();

        let parsed: ChatResponse =
            serde_json::from_slice(&bytes).map_err(|e| ChatError::Decode {
                detail: format!("{e} (received {} bytes)", bytes.len()),
            })?;

        let calls = parsed.tool_calls().to_vec();

        // A response requesting tools carries no text of its own, which is not an error.
        let content = match parsed.first_content() {
            Some(text) => text,
            None if !calls.is_empty() => String::new(),
            None => return Err(ChatError::NoContent),
        };

        let usage = parsed.usage();

        Ok(Completion {
            content: Labelled::new(content, label),
            model: parsed.model.unwrap_or_else(|| "unreported".to_string()),
            calls,
            usage,
        })
    }

    /// Send a chat completion request and read the reply as it arrives.
    ///
    /// Identical to [`AichatClient::complete`] in what it produces and in the gates it passes;
    /// `progress` is called as chunks land so a caller can show the reply arriving.
    ///
    /// What it receives is how much the model has written and the words written since the last
    /// call, still labelled. The words are untrusted model output and stay that way here: this
    /// crate mints nothing, and a caller that wants to read them needs a witness of its own,
    /// which the display gate is the only reasonable place to get. A caller with nowhere to draw
    /// them can ignore them and read the count.
    pub fn complete_streaming<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
        mut progress: impl FnMut(Progress),
    ) -> Result<Completion, ChatError> {
        let request = request.clone().streamed();
        let mut attempt = 1;
        loop {
            match self.stream_once(policy, &request, attempt, &mut progress) {
                Err(error) if worth_another_attempt(attempt, &error) => {
                    attempt += 1;
                    // Announced before the wait rather than after it, so the pause is explained
                    // while it is happening. Nothing of the abandoned attempt survives: the reply
                    // starts again from nothing, and the count says so.
                    progress(Progress {
                        written: Labelled::new("", Label::untrusted_public()),
                        output_tokens: 0,
                        counted_by_server: false,
                        attempt,
                    });
                    if !self.wait(backoff(attempt - 1)) {
                        return Err(ChatError::Cancelled);
                    }
                }
                result => return result,
            }
        }
    }

    /// Wait out a backoff, or give up on it when the caller says to stop.
    ///
    /// Slept in slices rather than in one go, because the pause between attempts is seconds long
    /// and a stop landing in the middle of one would otherwise wait the rest of it out, having
    /// nothing to stop but a sleep.
    ///
    /// Returns whether the wait finished, so a stop is answered rather than swallowed.
    fn wait(&self, how_long: Duration) -> bool {
        const SLICE: Duration = Duration::from_millis(50);

        let until = std::time::Instant::now() + how_long;
        loop {
            if self.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                return false;
            }
            let left = until.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return true;
            }
            std::thread::sleep(left.min(SLICE));
        }
    }

    /// One attempt at [`AichatClient::complete_streaming`].
    ///
    /// A failed attempt leaves nothing behind. The reply is reassembled from the frames of one
    /// stream, so a stream that stopped halfway cannot be continued by a second one: what it had
    /// written is dropped and the request is sent again whole.
    fn stream_once<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
        attempt: u32,
        progress: &mut impl FnMut(Progress),
    ) -> Result<Completion, ChatError> {
        // Before the request is built, let alone sent. A stop that landed while the last attempt
        // was failing is a stop, and spending a credential on a reply nobody wants is worse than
        // slow.
        if self.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
            return Err(ChatError::Cancelled);
        }

        let http = self.prepare(request)?.header("accept", "text/event-stream");

        let stream = self
            .egress
            .fetch_streaming(policy, http, Label::untrusted_public())?;
        let label = stream.label();

        // Read on a thread this one can walk away from.
        //
        // A read blocks for as long as the other end is quiet and cannot be interrupted, and the
        // longest quiet in a turn is the one before the model's first word: a model thinking is a
        // socket with nothing on it. Checked between chunks and no closer, a stop pressed then
        // could not be noticed until the model started writing, which is the moment somebody is
        // most likely to have pressed it.
        //
        // Nothing on this thread holds a policy, a workspace or a tool: it reads bytes and sends
        // them on. So a request walked away from leaves a socket to be dropped when the server
        // finishes or the connection times out, and nothing else.
        let (chunks, arriving) = std::sync::mpsc::sync_channel(CHUNKS_AHEAD);
        std::thread::spawn(move || {
            let mut stream = stream;
            loop {
                match stream.next_chunk() {
                    // A send that fails means the receiver is gone, which means the caller
                    // stopped: there is nobody left to read for.
                    Ok(Some(piece)) => {
                        if chunks.send(Ok(Some(piece))).is_err() {
                            return;
                        }
                    }
                    end => {
                        let _ = chunks.send(end);
                        return;
                    }
                }
            }
        });

        let mut decoder = SseDecoder::new();
        let mut accumulated = StreamAccumulator::new();

        loop {
            let piece = match arriving.recv_timeout(WAKE) {
                Ok(Ok(Some(piece))) => piece,
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(e.into()),
                // Nothing has arrived yet, which is the whole point of waiting with a limit: it
                // is the only chance to look at anything while a reply is still being waited for.
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if self.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                        return Err(ChatError::Cancelled);
                    }
                    continue;
                }
                // The reader is gone without having said the body ended, which is the silence a
                // connection that died leaves, and is answered as one below.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };

            // A stream is read and nothing else, so there is nothing part written to leave behind
            // by stopping here, and the caller is throwing the reply away regardless.
            if self.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                return Err(ChatError::Cancelled);
            }

            // The SSE envelope is transport structure, like the JSON envelope in `complete`: the
            // bytes are taken out to find where events begin and end, and the reply that comes out
            // is relabelled with exactly the label it arrived under.
            let (bytes, _) = piece.into_parts_for_decoding();

            // Where the reply stood before this chunk was folded in, so what the chunk added can
            // be handed on without sending the whole reply again on every frame of a long one.
            let written_before = accumulated.content().len();

            for payload in decoder.push(&bytes) {
                if payload == STREAM_DONE {
                    accumulated.mark_ended();
                    continue;
                }
                // A chunk that will not parse is skipped rather than failing the turn: servers
                // send keepalives and comments, and one unreadable frame should not discard a
                // reply that is otherwise arriving fine.
                let Ok(chunk) = serde_json::from_str::<ChatChunk>(&payload) else {
                    continue;
                };
                accumulated.push(chunk);
            }

            progress(Progress {
                written: Labelled::new(&accumulated.content()[written_before..], label),
                output_tokens: accumulated.output_tokens(),
                counted_by_server: accumulated.usage_is_reported(),
                attempt,
            });
        }

        // Checked before the reply is taken apart, because the question is about the stream and
        // not about what it managed to carry. A server that hangs up mid-reply leaves the same
        // end of input as one that finished, so without this a cut-off answer was returned as a
        // whole one: the tool call the model was in the middle of writing simply vanished, and
        // the turn ended on what looked like a considered reply.
        if !accumulated.ended() {
            return Err(ChatError::Incomplete);
        }

        let (content, model, calls, usage) = accumulated.finish();

        if content.is_empty() && calls.is_empty() {
            return Err(ChatError::NoContent);
        }

        Ok(Completion {
            content: Labelled::new(content, label),
            model: model.unwrap_or_else(|| "unreported".to_string()),
            calls,
            usage,
        })
    }
}

/// How far a streamed reply has got.
///
/// The words are borrowed from the reply being assembled and carry the label the stream arrived
/// under, so nothing here is a copy and nothing here is readable without a witness. A caller with
/// a screen has one; a caller without a screen never mints it and never sees the text.
/// Neither `Copy` nor comparable, because the labelled words are neither. That is the point:
/// a value that cannot be compared cannot be branched on by accident.
#[derive(Debug, Clone)]
pub struct Progress<'a> {
    /// What the model wrote since the last report, still labelled.
    ///
    /// The part rather than the whole, so drawing a reply as it arrives costs what the reply
    /// costs rather than the square of it.
    pub written: Labelled<&'a str>,
    /// Output tokens so far: the server's own figure once it has given one, and until then a count
    /// of the chunks that carried text.
    pub output_tokens: u64,
    /// Whether that figure is the server's rather than an estimate.
    ///
    /// Worth knowing at the point of display: an estimate presented as a billed figure would be
    /// the kind of number that looks like data and is not.
    pub counted_by_server: bool,
    /// Which attempt this is, counting from one.
    ///
    /// Above one, an earlier attempt failed in transit and this one restarted the reply. The
    /// count goes back to zero with it, so a display that only ever saw the number climb would
    /// otherwise show it falling for no stated reason.
    pub attempt: u32,
}

/// How many times one request is sent before its failure is the caller's.
///
/// Three because the failure this exists for is a connection that died while nobody was
/// looking, and a machine coming back from sleep needs a moment before its network works.
const ATTEMPTS: u32 = 3;

/// How long to wait after the first failure. Doubled for each attempt after that.
const BACKOFF: Duration = Duration::from_secs(1);

/// How often a reply that has not started arriving looks up to see whether it should stop.
///
/// The same interval the rest of the system waits on for the same question, and short enough that
/// a person cannot tell it from at once.
const WAKE: Duration = Duration::from_millis(50);

/// How many chunks may sit between the thread reading them and the one taking them apart.
///
/// Bounded, so a server faster than the decoder cannot pile a whole reply into a channel. Small,
/// because the decoder has never been the slow end.
const CHUNKS_AHEAD: usize = 16;

/// Whether a failed attempt should be repeated.
///
/// Only transport failures qualify, and only from the layer that knows what happened to the
/// connection. A reply that arrived and would not decode is not a connection problem, and
/// asking for it again would produce the same thing.
fn worth_another_attempt(attempt: u32, error: &ChatError) -> bool {
    if attempt >= ATTEMPTS {
        return false;
    }
    match error {
        ChatError::Egress(e) => e.is_transient(),
        // A reply that stopped early is a request that did not complete, whatever the socket
        // thought. Sending it again is the same remedy, and the partial is thrown away for the
        // same reason: half a reply cannot be continued by a second stream.
        ChatError::Incomplete => true,
        _ => false,
    }
}

fn backoff(failures: u32) -> Duration {
    BACKOFF * 2u32.pow(failures - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::Message;

    fn config() -> Config {
        Config::from_lookup(|key| {
            match key {
                "SERVICES_KEY_AICHAT" => Some("test-signing-key"),
                "BRAVE_SERVICES_KEY_ID" => Some("test-key-id"),
                "BRAVE_AI_CHAT_ENDPOINT" => Some("https://brave.example.invalid"),
                _ => None,
            }
            .map(str::to_string)
        })
        .expect("configured")
    }

    fn provider(models: &str) -> bravebot_config::provider::Provider {
        let text = format!(
            r#"{{"provider": {{"openrouter": {{
                "options": {{"baseURL": "https://openrouter.example.invalid/api/v1"}},
                "models": {models}
            }}}}}}"#
        );
        let serde_json::Value::Object(root) = serde_json::from_str(&text).expect("json") else {
            panic!("not an object");
        };
        bravebot_config::provider::Provider::all(&root)
            .pop()
            .expect("one provider")
    }

    fn request(model: &str) -> ChatRequest {
        ChatRequest::new(model, vec![Message::user("hello")])
    }

    fn body(http: &Request) -> serde_json::Value {
        serde_json::from_slice(http.body.as_ref().expect("a body")).expect("json")
    }

    fn header<'a>(http: &'a Request, name: &str) -> Option<&'a str> {
        http.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }

    /// A gateway is reached at its own host with its own credential. Signing it as though it were
    /// Brave's endpoint would send a bearer service to a service that has never heard of one.
    #[test]
    fn a_gateway_request_goes_to_the_configured_host_with_a_bearer_token() {
        let config = config();
        let egress = Egress::new();
        let provider = provider(r#"{"z-ai/glm-4.6": {}}"#);
        let http = AichatClient::new(&config, &egress)
            .for_gateway(&provider, "a-token")
            .prepare(&request("z-ai/glm-4.6"))
            .expect("prepared");

        assert_eq!(
            http.url,
            "https://openrouter.example.invalid/api/v1/chat/completions"
        );
        assert_eq!(header(&http, "authorization"), Some("Bearer a-token"));
        assert_eq!(header(&http, "digest"), None);
    }

    /// Nothing changes for Brave's endpoint, which is every existing request. A gateway is additive,
    /// and a client built without one still signs.
    #[test]
    fn a_request_without_a_gateway_is_still_signed() {
        let config = config();
        let egress = Egress::new();
        let http = AichatClient::new(&config, &egress)
            .prepare(&request("automatic"))
            .expect("prepared");

        assert_eq!(
            http.url,
            "https://brave.example.invalid/v1/chat/completions"
        );
        assert!(header(&http, "digest").is_some());
        assert!(
            header(&http, "authorization").is_some_and(|value| !value.starts_with("Bearer ")),
            "a signed request does not bearer-authenticate"
        );
    }

    /// The escape hatch the block exists for: a gateway's routing controls are its own invention, so
    /// they reach the body whole rather than through a schema that has to know what they mean.
    #[test]
    fn model_options_are_merged_into_the_request_body() {
        let config = config();
        let egress = Egress::new();
        let provider = provider(
            r#"{"anthropic/claude-sonnet-4.5": {"options": {
                "provider": {"order": ["amazon-bedrock"], "allow_fallbacks": false}
            }}}"#,
        );
        let http = AichatClient::new(&config, &egress)
            .for_gateway(&provider, "a-token")
            .prepare(&request("anthropic/claude-sonnet-4.5"))
            .expect("prepared");

        let body = body(&http);
        assert_eq!(
            body.get("provider"),
            Some(&serde_json::json!({"order": ["amazon-bedrock"], "allow_fallbacks": false}))
        );
        assert_eq!(
            body.get("model"),
            Some(&serde_json::json!("anthropic/claude-sonnet-4.5"))
        );
    }

    /// A configuration may add to a request and must not rewrite it. Letting `options` replace the
    /// model or the messages would let the file decide what a turn asked, which is not what a
    /// destination is for.
    #[test]
    fn model_options_cannot_overwrite_what_the_turn_built() {
        let config = config();
        let egress = Egress::new();
        let provider = provider(
            r#"{"m": {"options": {"model": "something/else", "messages": [], "stream": true}}}"#,
        );
        let http = AichatClient::new(&config, &egress)
            .for_gateway(&provider, "a-token")
            .prepare(&request("m"))
            .expect("prepared");

        let body = body(&http);
        assert_eq!(body.get("model"), Some(&serde_json::json!("m")));
        assert_eq!(
            body.get("messages")
                .and_then(|it| it.as_array())
                .map(Vec::len),
            Some(1)
        );
    }

    /// A model with nothing to add sends the body the turn built, so a gateway entry that states
    /// only a window is not also a request modifier.
    #[test]
    fn a_model_with_no_options_adds_nothing_to_the_body() {
        let config = config();
        let egress = Egress::new();
        let provider =
            provider(r#"{"z-ai/glm-4.6": {"limit": {"context": 131072, "output": 8192}}}"#);
        let http = AichatClient::new(&config, &egress)
            .for_gateway(&provider, "a-token")
            .prepare(&request("z-ai/glm-4.6"))
            .expect("prepared");

        let body = body(&http);
        let keys: Vec<&String> = body.as_object().expect("an object").keys().collect();
        assert_eq!(keys, ["messages", "model"]);
    }
}
