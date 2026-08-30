//! Client for the OpenAI-compatible aichat backend.
//!
//! Targets `POST /v1/chat/completions`. Despite the `/v1` prefix that is the **v2**
//! API: the server infers the version from the path, so there is no `/v2/` route to
//! construct. `/v1/conversation` is the older, deprecated surface.
//!
//! Requests are signed rather than bearer-authenticated; see [`bravebot_signing`]. All
//! traffic goes through [`bravebot_net::Egress`] so the policy gate sees it, and the model
//! reported in the response is preserved because the server may substitute a different
//! one than was requested.

pub mod models;
pub mod protocol;

use bravebot_config::Config;
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
}

impl<'a> AichatClient<'a> {
    pub fn new(config: &'a Config, egress: &'a Egress) -> Self {
        Self {
            config,
            egress,
            subscription: None,
        }
    }

    /// Send requests on the premium tier, spending a credential on each.
    pub fn with_subscription(mut self, subscription: &'a mut dyn Subscription) -> Self {
        self.subscription = Some(subscription);
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
                    std::thread::sleep(backoff(attempt - 1));
                }
                result => return result,
            }
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
        let body = serde_json::to_vec(request).map_err(|e| ChatError::Encode(e.to_string()))?;

        let headers =
            bravebot_signing::sign(self.config.signing_key.expose(), &self.config.key_id, &body);

        let (url, credential) = self.route()?;

        let mut http = Request::post(url, body)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .header("digest", &headers.digest)
            .header("authorization", &headers.authorization);

        if let Some(credential) = credential {
            http = http.header(
                "cookie",
                format!("{}={}", credential.cookie_name, credential.cookie_value),
            );
        }

        let mut stream = self
            .egress
            .fetch_streaming(policy, http, Label::untrusted_public())?;
        let label = stream.label();

        let mut decoder = SseDecoder::new();
        let mut accumulated = StreamAccumulator::new();

        while let Some(piece) = stream.next_chunk()? {
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
