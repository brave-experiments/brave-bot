//! Client for models on AWS Bedrock.
//!
//! The second backend. It answers the same questions as [`bravebot_aichat`] and returns the same
//! [`Completion`], so a caller chooses between them once and the turn loop is unchanged. What differs
//! is underneath: Bedrock's own Converse API rather than an OpenAI-compatible one, SigV4 signatures
//! over short-lived credentials rather than an HMAC over a body digest, and a binary event-stream
//! framing rather than server-sent events.
//!
//! Converse states one body for every provider Bedrock hosts, so a tier may name a model from any
//! of them and nothing here has to recognise which.
//!
//! Every request goes through [`bravebot_net::Egress`], so the policy gate sees this traffic exactly
//! as it sees the other backend's. The reply is labelled untrusted-public and nothing here reads it:
//! this crate speaks the wire protocol and mints no witness.

pub mod credentials;
pub mod eventstream;
pub mod protocol;

use bravebot_aichat::protocol::{ChatRequest, Usage};
use bravebot_aichat::{Completion, Progress};
use bravebot_config::bedrock::Bedrock;
use bravebot_core::cancel::Cancel;
use bravebot_core::event::Sink;
use bravebot_core::label::Label;
use bravebot_core::policy::Policy;
use bravebot_core::value::Labelled;
use bravebot_net::{Egress, EgressError, Request};
use eventstream::FrameDecoder;
use protocol::StreamEvent;
use std::fmt;
use std::time::Duration;

/// The service name SigV4 signs for.
const SERVICE: &str = "bedrock";

#[derive(Debug)]
pub enum BedrockError {
    /// Credentials could not be resolved, even after a sign-in was offered.
    Credentials(credentials::CredentialError),
    /// The request could not be serialised.
    Encode(String),
    /// The response was not the expected shape.
    Decode { detail: String },
    /// The request never left, or failed in transit.
    Egress(EgressError),
    /// The reply's framing was corrupt, so where it ended is unknown.
    Frame(eventstream::FrameError),
    /// A well-formed response carrying no usable content.
    NoContent,
    /// The stream stopped without the service saying the reply was over.
    Incomplete,
    /// The service said, part way through the reply, that it was not going to finish it.
    ///
    /// Named by the failure the service reported rather than by a status: the status was sent and
    /// accepted before the reply began, so this is the only thing that says why it stopped.
    Reported { kind: String },
    /// The model was cut off at the token ceiling.
    ///
    /// Distinct from [`BedrockError::Incomplete`], which is a connection that died: this reply ended
    /// because it ran out of room, so sending it again unchanged produces the same result.
    TooLong,
    /// No model is configured, so there is nothing to send to.
    NoModel,
    /// The caller asked for the reply to stop arriving.
    Cancelled,
}

impl fmt::Display for BedrockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Credentials(e) => write!(f, "{e}"),
            Self::Encode(detail) => write!(f, "could not encode the request: {detail}"),
            Self::Decode { detail } => write!(f, "unexpected response: {detail}"),
            // Named as what it is, because the bare status says nothing about the remedy. A refused
            // credential reads as an unexplained HTTP failure otherwise, and the thing that fixes it
            // is a sign-in nobody was told to do.
            Self::Egress(_) if self.is_credential_refused() => f.write_str(
                "AWS refused the credentials this request was signed with. The session has most \
                 likely expired: sign in again, and the next turn will offer to",
            ),
            Self::Egress(e) => write!(f, "{e}"),
            Self::Frame(e) => write!(f, "{e}"),
            Self::NoContent => f.write_str("the response contained no message content"),
            Self::Incomplete => {
                f.write_str("the reply stopped before the service said it was finished")
            }
            Self::Reported { kind } => {
                write!(f, "AWS stopped the reply part way through and reported {kind}")
            }
            Self::TooLong => f.write_str(
                "the model reached its output limit before finishing. Ask for less in one turn",
            ),
            Self::NoModel => f.write_str(
                "no Bedrock model is configured. Set ANTHROPIC_DEFAULT_OPUS_MODEL (or the sonnet or \
                 haiku equivalent) in ~/.bravebot/settings.json",
            ),
            Self::Cancelled => f.write_str("the reply was stopped while it was arriving"),
        }
    }
}

impl std::error::Error for BedrockError {}

/// The status AWS answers a request it will not accept the contents of.
///
/// A validation refusal, which is what a model answers a parameter it does not define. Not worth
/// sending again unchanged, and the one thing worth changing is the part of the request nobody
/// asked for.
const REFUSED_CONTENTS_STATUS: u16 = 400;

/// The status a request carrying cache breakpoints is refused with.
///
/// A model that does not do prompt caching answers 403, not the validation status a rejected
/// parameter gets, and it is the same 403 an expired credential gets. Nothing in the body is read
/// to tell the two apart: the breakpoints are dropped and the request sent again, and a failure
/// that survives that was the credential after all.
const REFUSED_CACHING_STATUS: u16 = 403;

/// The statuses AWS answers a credential it will not accept with.
///
/// 401 is a credential it did not recognise and 403 one it recognised and refused. Neither is worth
/// sending again unchanged, which is why they are absent from what the egress layer calls transient:
/// what they are worth is a sign-in.
const REFUSED_STATUSES: [u16; 2] = [401, 403];

impl BedrockError {
    /// Whether AWS refused the credential this request was signed with.
    ///
    /// Asked so a caller can offer the remedy. A credential the AWS CLI produced happily can still be
    /// rejected here: it caches the role credentials it derived, so an expired session keeps
    /// answering locally with something the service has stopped accepting, and expiry can also fall
    /// between the start of a run and a later request in it.
    ///
    /// Not a decision taken from content. A status is the transport's own report, and nothing in the
    /// body is read to reach it.
    pub fn is_credential_refused(&self) -> bool {
        matches!(
            self,
            Self::Egress(EgressError::Status { status, .. }) if REFUSED_STATUSES.contains(status)
        )
    }

    /// Whether AWS refused this request on what it contained.
    ///
    /// Not a decision taken from content, for the same reason the one above is not: a status is the
    /// transport's own report, and nothing in the body is read to reach it.
    pub fn is_refused_on_contents(&self) -> bool {
        matches!(
            self,
            Self::Egress(EgressError::Status { status, .. }) if *status == REFUSED_CONTENTS_STATUS
        )
    }

    /// Whether this could be AWS refusing the cache breakpoints a request carried.
    ///
    /// Could be, rather than is: the status a model without prompt caching answers is the one an
    /// expired credential answers too, and which it was is settled by asking again without them
    /// rather than by reading the body.
    pub fn may_refuse_caching(&self) -> bool {
        matches!(
            self,
            Self::Egress(EgressError::Status { status, .. }) if *status == REFUSED_CACHING_STATUS
        )
    }
}

impl From<EgressError> for BedrockError {
    fn from(value: EgressError) -> Self {
        match value {
            // Reported as the stop it is rather than as a transport failure, so a caller reading
            // the outcome cannot mistake a withdrawn request for a connection that broke.
            EgressError::Stopped { .. } => Self::Cancelled,
            other => Self::Egress(other),
        }
    }
}

impl From<eventstream::FrameError> for BedrockError {
    fn from(value: eventstream::FrameError) -> Self {
        Self::Frame(value)
    }
}

impl From<credentials::CredentialError> for BedrockError {
    fn from(value: credentials::CredentialError) -> Self {
        Self::Credentials(value)
    }
}

pub struct BedrockClient<'a> {
    config: &'a Bedrock,
    egress: &'a Egress,
    cancel: Option<Cancel>,
    /// Whether requests still carry cache breakpoints.
    ///
    /// True until a model refuses one, which is the only way to find out that it does not do
    /// prompt caching: an inference-profile ARN does not say which model is behind it.
    breakpoints: bool,
    /// Whether requests still carry the effort level a turn was given.
    ///
    /// True until a model refuses the field, which is the only way to find out that it does not
    /// define one. The field belongs to the model's own provider rather than to this API, so a
    /// model from another provider refuses it by name.
    effort: bool,
}

impl<'a> BedrockClient<'a> {
    pub fn new(config: &'a Bedrock, egress: &'a Egress) -> Self {
        Self {
            config,
            egress,
            cancel: None,
            breakpoints: true,
            effort: true,
        }
    }

    /// Stop reading a streamed reply as soon as this says to.
    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Send a request and wait for the whole reply.
    pub fn complete<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
    ) -> Result<Completion, BedrockError> {
        let mut attempt = 1;
        let mut probed = false;
        loop {
            match self.complete_once(policy, request) {
                // Sent again immediately rather than after a wait: nothing is busy, the request was
                // simply carrying something this model does not take.
                Err(error) if self.worth_dropping_breakpoints(&error) => {
                    self.breakpoints = false;
                    probed = true;
                }
                Err(error) if self.worth_dropping_effort(&error) => {
                    self.effort = false;
                    probed = true;
                }
                Err(error) if worth_another_attempt(attempt, &error) => {
                    std::thread::sleep(backoff(attempt));
                    attempt += 1;
                }
                result => {
                    self.probe_settled(probed, result.is_err());
                    return result;
                }
            }
        }
    }

    /// Whether this failure is worth sending the same request again without its cache breakpoints.
    ///
    /// Only once, since the answer is remembered, and only for a refusal on the request's contents.
    /// The breakpoints are the one part of a request nobody asked for, so a service that refuses it
    /// is worth asking without them before the failure is anybody else's.
    fn worth_dropping_breakpoints(&self, error: &BedrockError) -> bool {
        self.breakpoints && error.may_refuse_caching()
    }

    /// Whether this failure is worth sending the same request again without its effort level.
    ///
    /// The level is the other part of a request nobody asked for, and the only one a validation
    /// refusal can be about once the breakpoints are gone.
    fn worth_dropping_effort(&self, error: &BedrockError) -> bool {
        self.effort && error.is_refused_on_contents()
    }

    /// Settle what a probe found, once the request it was part of has finished one way or the other.
    ///
    /// A probe that answered leaves the breakpoints dropped, which is the model saying it does not
    /// read them. A probe that failed as well says they were not what the service refused, so they
    /// go back: a request can be refused on its contents for reasons that have nothing to do with
    /// them, and a session that gave them up for one of those pays full price for a prefix the
    /// service would have read once, every round, for the rest of its life.
    fn probe_settled(&mut self, probed: bool, failed: bool) {
        if probed && failed {
            self.breakpoints = true;
            self.effort = true;
        }
    }

    fn complete_once<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
    ) -> Result<Completion, BedrockError> {
        let (http, model) = self.build(request, false)?;
        let response = self.egress.fetch(policy, http, Label::untrusted_public())?;

        // The envelope is protocol, like the JSON envelope in the other backend: the bytes come out
        // to find the reply inside, and the reply is relabelled with exactly the label it arrived
        // under.
        let (bytes, label) = response.body.into_parts_for_decoding();

        let parsed: protocol::ConverseResponse =
            serde_json::from_slice(&bytes).map_err(|e| BedrockError::Decode {
                detail: format!("{e} (received {} bytes)", bytes.len()),
            })?;

        if parsed.stop_reason.as_deref() == Some(protocol::STOP_REASON_MAX_TOKENS) {
            return Err(BedrockError::TooLong);
        }

        let blocks = parsed
            .output
            .and_then(|output| output.message)
            .map(|message| message.content)
            .unwrap_or_default();
        if blocks.iter().any(protocol::ReplyBlock::is_unreadable_call) {
            return Err(BedrockError::Decode {
                detail: "the reply named a tool call in a shape this does not read".to_string(),
            });
        }

        let (content, calls) = protocol::parts_of(&blocks);
        if content.is_empty() && calls.is_empty() {
            return Err(BedrockError::NoContent);
        }

        Ok(Completion {
            content: Labelled::new(content, label),
            // This API does not name the model back, so the one the request asked for is the one
            // that answered.
            model,
            calls,
            usage: parsed.usage.map(Usage::from).unwrap_or_default(),
        })
    }

    /// Send a request and read the reply as it arrives.
    ///
    /// Identical to [`BedrockClient::complete`] in what it produces and the gates it passes.
    /// `progress` is called as events land so a caller can show the reply arriving.
    pub fn complete_streaming<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
        mut progress: impl FnMut(Progress),
    ) -> Result<Completion, BedrockError> {
        let mut attempt = 1;
        let mut probed = false;
        loop {
            match self.stream_once(policy, request, attempt, &mut progress) {
                Err(error) if self.worth_dropping_breakpoints(&error) => {
                    self.breakpoints = false;
                    probed = true;
                }
                Err(error) if self.worth_dropping_effort(&error) => {
                    self.effort = false;
                    probed = true;
                }
                Err(error) if worth_another_attempt(attempt, &error) => {
                    attempt += 1;
                    // Announced before the wait rather than after it, so the pause is explained
                    // while it is happening. Nothing of the abandoned attempt survives.
                    progress(Progress {
                        written: Labelled::new("", Label::untrusted_public()),
                        output_tokens: 0,
                        counted_by_server: false,
                        attempt,
                    });
                    if !self.wait(backoff(attempt - 1)) {
                        return Err(BedrockError::Cancelled);
                    }
                }
                result => {
                    self.probe_settled(probed, result.is_err());
                    return result;
                }
            }
        }
    }

    fn stream_once<S: Sink>(
        &mut self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
        attempt: u32,
        progress: &mut impl FnMut(Progress),
    ) -> Result<Completion, BedrockError> {
        // Before the request is built, let alone sent. A stop that landed while the last attempt was
        // failing is still a stop.
        if self.cancelled() {
            return Err(BedrockError::Cancelled);
        }

        let (http, model) = self.build(request, true)?;
        let stream = self.egress.fetch_streaming(
            policy,
            http,
            Label::untrusted_public(),
            self.cancel.as_ref(),
        )?;
        let label = stream.label();

        // Read on a thread this one can walk away from, for the same reason the other backend does:
        // a read blocks for as long as the service is quiet, and the longest quiet in a turn is the
        // one before the model's first word. Nothing on this thread holds a policy, a workspace or a
        // tool, so a request walked away from leaves only a socket to be dropped.
        let (chunks, arriving) = std::sync::mpsc::sync_channel(CHUNKS_AHEAD);
        std::thread::spawn(move || {
            let mut stream = stream;
            loop {
                match stream.next_chunk() {
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

        let mut decoder = FrameDecoder::new();
        let mut reply = Reply::default();

        loop {
            let piece = match arriving.recv_timeout(WAKE) {
                Ok(Ok(Some(piece))) => piece,
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(e.into()),
                // Nothing has arrived yet, which is the only chance to look at anything while a
                // reply is still being waited for.
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if self.cancelled() {
                        return Err(BedrockError::Cancelled);
                    }
                    continue;
                }
                // The reader is gone without having said the body ended, which is the silence a dead
                // connection leaves, and is answered as one below.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };

            if self.cancelled() {
                return Err(BedrockError::Cancelled);
            }

            let (bytes, _) = piece.into_parts_for_decoding();
            let written_before = reply.text.len();

            for event in decoder.push(&bytes)? {
                match event {
                    // An event this does not model, or one whose body will not parse, is skipped
                    // rather than failing the turn: the framing was sound, so the position in the
                    // stream is known, and the API sends events that say nothing this needs.
                    eventstream::Event::Named { name, payload } => {
                        if let Some(event) = protocol::stream_event(&name, &payload) {
                            reply.absorb(event);
                        }
                    }
                    // Reported rather than read past. A reply the service abandoned ends the same
                    // way a dead connection does, so without this the cause is replaced by "the
                    // reply was cut off" and a refusal nothing can fix is asked for twice more.
                    eventstream::Event::Failed { kind } => {
                        return Err(BedrockError::Reported { kind });
                    }
                }
            }

            progress(Progress {
                written: Labelled::new(&reply.text[written_before..], label),
                output_tokens: reply.usage.completion_tokens,
                counted_by_server: reply.counted,
                attempt,
            });
        }

        // Checked before the reply is taken apart, because the question is about the stream and not
        // about what it carried. A service that hangs up mid-reply leaves the same end of input as
        // one that finished, so without this a cut-off answer is returned as a whole one and the tool
        // call the model was writing simply vanishes.
        if !reply.ended || decoder.is_mid_frame() {
            return Err(BedrockError::Incomplete);
        }

        if reply.unreadable_call {
            return Err(BedrockError::Decode {
                detail: "the reply named a tool call in a shape this does not read".to_string(),
            });
        }

        if reply.stop_reason.as_deref() == Some(protocol::STOP_REASON_MAX_TOKENS) {
            return Err(BedrockError::TooLong);
        }

        let calls = reply.calls();
        if reply.text.is_empty() && calls.is_empty() {
            return Err(BedrockError::NoContent);
        }

        Ok(Completion {
            content: Labelled::new(reply.text, label),
            // This API does not name the model back, so the one the request asked for is the one
            // that answered.
            model,
            calls,
            usage: reply.usage,
        })
    }

    /// The signed request for one attempt, and the model it names.
    ///
    /// Credentials are resolved here rather than held, because a session expires during a run and a
    /// key read once at startup stops working part way through.
    fn build(
        &self,
        request: &ChatRequest,
        streaming: bool,
    ) -> Result<(Request, String), BedrockError> {
        let model = self.model_for(request)?;

        let converse = protocol::request_from(&request.messages, request.tools.as_deref())
            .with_effort(request.effort.filter(|_| self.effort));
        let converse = if self.breakpoints {
            converse
        } else {
            converse.without_breakpoints()
        };
        let body =
            serde_json::to_vec(&converse).map_err(|e| BedrockError::Encode(e.to_string()))?;

        let resolved = credentials::resolve(self.config.profile.as_deref())?;

        let url = self.config.converse_url(&model, streaming);
        let host = self.config.host();
        let path = path_of(&url);

        let signed = bravebot_signing::sigv4::sign_post(
            bravebot_signing::sigv4::Credentials {
                access_key_id: &resolved.access_key_id,
                secret_access_key: resolved.secret_access_key.expose(),
                session_token: resolved.session_token.as_ref().map(|t| t.expose()),
            },
            &self.config.region,
            SERVICE,
            &host,
            &path,
            &body,
            now(),
        );

        let mut http = Request::post(url, body)
            .header("content-type", "application/json")
            .header("host", host)
            .header("x-amz-date", &signed.date)
            .header("x-amz-content-sha256", &signed.content_sha256)
            .header("authorization", &signed.authorization);

        if let Some(token) = &signed.security_token {
            http = http.header("x-amz-security-token", token);
        }

        Ok((http, model))
    }

    /// Which model this request names.
    ///
    /// Ordinarily the name it arrived with: the backend is selected by asking which one offers the
    /// model, so a request reaching here names a configured tier. The fallback is for a caller that
    /// did not ask, since Bedrock rejects an unknown model rather than substituting one, and a name
    /// this configuration does not have is better replaced here than sent.
    fn model_for(&self, request: &ChatRequest) -> Result<String, BedrockError> {
        if self.config.offers(&request.model) {
            return Ok(request.model.clone());
        }
        self.config
            .default_model()
            .map(str::to_string)
            .ok_or(BedrockError::NoModel)
    }

    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(Cancel::is_cancelled)
    }

    /// Wait out a backoff, or give up on it when the caller says to stop.
    ///
    /// Slept in slices, because a stop landing in the middle of a seconds-long pause would otherwise
    /// wait the rest of it out with nothing to stop but a sleep.
    fn wait(&self, how_long: Duration) -> bool {
        const SLICE: Duration = Duration::from_millis(50);

        let until = std::time::Instant::now() + how_long;
        loop {
            if self.cancelled() {
                return false;
            }
            let left = until.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return true;
            }
            std::thread::sleep(left.min(SLICE));
        }
    }
}

/// A reply being assembled from the events of one stream.
#[derive(Debug, Default)]
struct Reply {
    text: String,
    /// Tool calls by block index, since their arguments arrive in pieces across events.
    calls: Vec<(usize, String, String, String)>,
    usage: Usage,
    /// Whether the count is the service's rather than a tally of what arrived.
    counted: bool,
    ended: bool,
    stop_reason: Option<String>,
    /// Whether a block opened as a tool call this could not read.
    ///
    /// Kept rather than failed on the spot so the stream is still drained: the reply is refused
    /// once it has ended, in the same place a reply that arrived whole is.
    unreadable_call: bool,
}

impl Reply {
    fn absorb(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::ContentBlockStart { index, start } => {
                match start {
                    // The opening event names the call and nothing else; every byte of its
                    // arguments arrives in the deltas that follow.
                    protocol::BlockStart::ToolUse { tool_use } => {
                        self.calls.push((
                            index,
                            tool_use.tool_use_id,
                            tool_use.name,
                            String::new(),
                        ));
                    }
                    protocol::BlockStart::UnreadableToolUse { .. } => self.unreadable_call = true,
                    protocol::BlockStart::Other(_) => {}
                }
            }
            StreamEvent::ContentBlockDelta { index, delta } => match delta {
                protocol::Delta::Text { text } => {
                    self.text.push_str(&text);
                    // A tally until the service reports its own, so a reply in flight can show
                    // something rather than zero.
                    if !self.counted {
                        self.usage.completion_tokens += 1;
                    }
                }
                protocol::Delta::ToolUse { tool_use } => {
                    if let Some(call) = self.calls.iter_mut().find(|(at, ..)| *at == index) {
                        call.3.push_str(&tool_use.input);
                    }
                }
                protocol::Delta::Other(_) => {}
            },
            StreamEvent::MessageStop { stop_reason } => {
                self.ended = true;
                if stop_reason.is_some() {
                    self.stop_reason = stop_reason;
                }
            }
            // Both halves of the count arrive together at the end of the stream, so this replaces
            // the running tally rather than adding to it.
            StreamEvent::Metadata { usage } => {
                if let Some(usage) = usage {
                    self.usage = Usage::from(usage);
                    self.counted = true;
                }
            }
        }
    }

    /// The calls this reply asked for, in the shape the agent expects.
    fn calls(&self) -> Vec<bravebot_aichat::protocol::ToolCall> {
        use bravebot_aichat::protocol::{ToolCall, ToolCallFunction};

        self.calls
            .iter()
            .map(|(_, id, name, arguments)| ToolCall {
                id: Some(id.clone()),
                function: ToolCallFunction {
                    name: name.clone(),
                    // An empty argument stream means a call with no arguments, which is `{}` rather
                    // than nothing: the turn loop parses this, and an empty string is not JSON.
                    arguments: Some(if arguments.is_empty() {
                        "{}".to_string()
                    } else {
                        arguments.clone()
                    }),
                },
            })
            .collect()
    }
}

/// The path of a URL, for signing.
///
/// The signature covers the path exactly as sent, so this takes it from the URL that will be
/// requested rather than rebuilding it.
fn path_of(url: &str) -> String {
    url.split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|at| rest[at..].to_string()))
        .unwrap_or_else(|| "/".to_string())
}

/// Seconds since the Unix epoch, for the signature's timestamp.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// The failures AWS reports mid-reply that a second attempt could get past.
///
/// Everything else it names is a property of the request, which a second identical one has too.
const TRANSIENT_REPORTS: [&str; 4] = [
    "throttlingException",
    "modelStreamErrorException",
    "internalServerException",
    "serviceUnavailableException",
];

/// How many times one request is sent before its failure is the caller's.
const ATTEMPTS: u32 = 3;

/// How long to wait after the first failure. Doubled for each attempt after that.
const BACKOFF: Duration = Duration::from_secs(1);

/// How often a reply that has not started arriving looks up to see whether it should stop.
const WAKE: Duration = Duration::from_millis(50);

/// How many chunks may sit between the thread reading them and the one taking them apart.
const CHUNKS_AHEAD: usize = 16;

/// Whether a failed attempt should be repeated.
///
/// Only transport failures qualify. A reply that arrived and would not decode is not a connection
/// problem, and asking again produces the same thing. Nor is an expired credential: that is fixed by
/// signing in, which resolving them already did.
fn worth_another_attempt(attempt: u32, error: &BedrockError) -> bool {
    if attempt >= ATTEMPTS {
        return false;
    }
    match error {
        BedrockError::Egress(e) => e.is_transient(),
        // The service's own name for what went wrong. A fault or a busy service is worth asking
        // again; a request it refuses on its contents is refused the same way every time.
        BedrockError::Reported { kind } => TRANSIENT_REPORTS.contains(&kind.as_str()),
        // A reply that stopped early is a request that did not complete, whatever the socket
        // thought. The partial is thrown away for the same reason: half a reply cannot be continued
        // by a second stream.
        BedrockError::Incomplete => true,
        // Corrupt framing means the position in the stream is lost, and the whole reply has to be
        // asked for again. The cause is a damaged connection, which is worth another attempt.
        BedrockError::Frame(_) => true,
        _ => false,
    }
}

fn backoff(failures: u32) -> Duration {
    BACKOFF * 2u32.pow(failures - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_config::env_var;

    /// The event that opens a tool call, which names it and carries none of its arguments.
    fn opening(id: &str, name: &str) -> protocol::BlockStart {
        protocol::BlockStart::ToolUse {
            tool_use: protocol::ToolUseStart {
                tool_use_id: id.to_string(),
                name: name.to_string(),
            },
        }
    }

    /// One fragment of a tool call's arguments.
    fn arguments(piece: &str) -> protocol::Delta {
        protocol::Delta::ToolUse {
            tool_use: protocol::ToolUseDelta {
                input: piece.to_string(),
            },
        }
    }

    /// A refusal carrying one status, as the egress layer reports one.
    fn refusal(status: u16) -> BedrockError {
        BedrockError::Egress(EgressError::Status {
            url: "https://bedrock-runtime.us-west-2.amazonaws.com/model/x/converse".to_string(),
            status,
        })
    }

    fn config() -> Bedrock {
        Bedrock::from_lookup(|name| {
            match name {
                env_var::USE_BEDROCK => Some("1"),
                env_var::AWS_REGION => Some("us-west-2"),
                env_var::BEDROCK_OPUS_MODEL => Some("opus-arn"),
                env_var::BEDROCK_HAIKU_MODEL => Some("haiku-arn"),
                _ => None,
            }
            .map(str::to_string)
        })
        .expect("configured")
    }

    /// A credential the CLI produced happily can still be refused here: it caches the role
    /// credentials it derived, so an expired session keeps answering locally with something AWS has
    /// stopped accepting. The bare status said nothing about the remedy, which is a sign-in.
    #[test]
    fn a_refused_credential_says_so_rather_than_reporting_a_status() {
        for status in [401, 403] {
            let error = BedrockError::Egress(EgressError::Status {
                url: "https://bedrock-runtime.us-west-2.amazonaws.com/model/x/converse".to_string(),
                status,
            });
            assert!(error.is_credential_refused(), "{status} was not recognised");
            let said = error.to_string();
            assert!(said.contains("AWS"), "{said}");
            assert!(said.contains("sign in"), "{said}");
            assert!(!said.contains(&status.to_string()), "{said}");
        }
    }

    /// Every other failure keeps its own account of itself. Reported as a refused credential, a
    /// server that was merely unwell would send somebody to sign in for nothing.
    #[test]
    fn another_failing_status_is_not_read_as_a_refused_credential() {
        for status in [400, 404, 429, 500, 503] {
            let error = BedrockError::Egress(EgressError::Status {
                url: "https://bedrock-runtime.us-west-2.amazonaws.com/model/x/invoke".to_string(),
                status,
            });
            assert!(
                !error.is_credential_refused(),
                "{status} was read as a refusal"
            );
            assert!(error.to_string().contains(&status.to_string()));
        }
    }

    /// The signature covers the path as sent. Signing a different one is a rejected request.
    #[test]
    fn the_signed_path_is_the_one_the_request_asks_for() {
        assert_eq!(
            path_of("https://host.invalid/model/abc/converse"),
            "/model/abc/converse"
        );
        assert_eq!(path_of("https://host.invalid/"), "/");
        assert_eq!(path_of("https://host.invalid"), "/");
    }

    /// A remembered choice outlives the settings that made it reachable. Bedrock rejects an unknown
    /// model rather than substituting one, so a stale name must fall back here instead of failing at
    /// the far end.
    #[test]
    fn a_model_that_is_no_longer_configured_falls_back_to_the_default() {
        let config = config();
        let egress = Egress::new();
        let client = BedrockClient::new(&config, &egress);

        let stale = ChatRequest::new("a-model-that-was-removed", vec![]);
        assert_eq!(client.model_for(&stale).expect("a model"), "opus-arn");

        let known = ChatRequest::new("haiku-arn", vec![]);
        assert_eq!(client.model_for(&known).expect("a model"), "haiku-arn");
    }

    /// A configuration naming no model has nothing to send to, and saying so beats inventing an ARN.
    #[test]
    fn no_configured_model_is_an_error_that_says_what_to_set() {
        let config = Bedrock::from_lookup(|name| {
            match name {
                env_var::USE_BEDROCK => Some("1"),
                env_var::AWS_REGION => Some("us-west-2"),
                _ => None,
            }
            .map(str::to_string)
        })
        .expect("configured");
        let egress = Egress::new();
        let client = BedrockClient::new(&config, &egress);

        let error = client
            .model_for(&ChatRequest::new("anything", vec![]))
            .expect_err("no model configured");
        assert!(matches!(error, BedrockError::NoModel));
        assert!(error.to_string().contains("ANTHROPIC_DEFAULT_OPUS_MODEL"));
    }

    /// The reply is assembled from the events of one stream, and text arriving in pieces is one
    /// answer.
    #[test]
    fn streamed_text_is_assembled_in_order() {
        let mut reply = Reply::default();
        for piece in ["Hello ", "world"] {
            reply.absorb(StreamEvent::ContentBlockDelta {
                index: 0,
                delta: protocol::Delta::Text { text: piece.into() },
            });
        }
        assert_eq!(reply.text, "Hello world");
    }

    /// Tool arguments arrive as JSON in pieces. Parsed before they are whole they are a syntax error,
    /// so they have to be concatenated first.
    #[test]
    fn streamed_tool_arguments_are_concatenated_before_use() {
        let mut reply = Reply::default();
        reply.absorb(StreamEvent::ContentBlockStart {
            index: 1,
            start: opening("call-1", "read_file"),
        });
        for piece in [r#"{"path""#, r#":"src/"#, r#"lib.rs"}"#] {
            reply.absorb(StreamEvent::ContentBlockDelta {
                index: 1,
                delta: arguments(piece),
            });
        }

        let calls = reply.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id.as_deref(), Some("call-1"));
        assert_eq!(
            calls[0].function.arguments.as_deref(),
            Some(r#"{"path":"src/lib.rs"}"#)
        );
    }

    /// Two calls in one reply have their pieces interleaved by block index, and mixing them produces
    /// two calls with corrupt arguments.
    #[test]
    fn interleaved_arguments_stay_with_their_own_call() {
        let mut reply = Reply::default();
        for (index, id) in [(0usize, "first"), (1usize, "second")] {
            reply.absorb(StreamEvent::ContentBlockStart {
                index,
                start: opening(id, "read_file"),
            });
        }
        for (index, piece) in [
            (0usize, r#"{"a""#),
            (1usize, r#"{"b""#),
            (0usize, r#":1}"#),
            (1usize, r#":2}"#),
        ] {
            reply.absorb(StreamEvent::ContentBlockDelta {
                index,
                delta: arguments(piece),
            });
        }

        let calls = reply.calls();
        assert_eq!(calls[0].function.arguments.as_deref(), Some(r#"{"a":1}"#));
        assert_eq!(calls[1].function.arguments.as_deref(), Some(r#"{"b":2}"#));
    }

    /// A call with no arguments is `{}`, not an empty string: the turn loop parses this field, and
    /// an empty string is not JSON.
    #[test]
    fn a_call_with_no_arguments_is_an_empty_object() {
        let mut reply = Reply::default();
        reply.absorb(StreamEvent::ContentBlockStart {
            index: 0,
            start: opening("call-1", "list"),
        });
        assert_eq!(reply.calls()[0].function.arguments.as_deref(), Some("{}"));
    }

    /// Both halves of the count arrive in the one event at the end of the stream, and a reply that
    /// kept only one of them reports half of what the turn cost. The cached tokens belong to the
    /// prompt they were sent as, whoever ended up reading them.
    #[test]
    fn both_halves_of_the_cost_survive_the_stream() {
        let mut reply = Reply::default();
        reply.absorb(StreamEvent::MessageStop {
            stop_reason: Some("end_turn".into()),
        });
        reply.absorb(StreamEvent::Metadata {
            usage: Some(protocol::BedrockUsage {
                input_tokens: 100,
                output_tokens: 42,
                cache_read_input_tokens: 800,
                cache_write_input_tokens: 100,
            }),
        });

        assert_eq!(reply.usage.prompt_tokens, 1_000);
        assert_eq!(reply.usage.completion_tokens, 42);
        assert_eq!(reply.usage.total(), 1_042);
        assert!(reply.counted, "the service reported its own figure");
    }

    /// Until the service reports a figure, a count of what arrived is shown so a reply in flight
    /// does not read as costing nothing.
    #[test]
    fn an_estimate_stands_in_until_the_service_reports_a_count() {
        let mut reply = Reply::default();
        for _ in 0..3 {
            reply.absorb(StreamEvent::ContentBlockDelta {
                index: 0,
                delta: protocol::Delta::Text { text: "x".into() },
            });
        }
        assert_eq!(reply.usage.completion_tokens, 3);
        assert!(!reply.counted, "not the service's own figure");

        reply.absorb(StreamEvent::Metadata {
            usage: Some(protocol::BedrockUsage {
                input_tokens: 0,
                output_tokens: 99,
                cache_read_input_tokens: 0,
                cache_write_input_tokens: 0,
            }),
        });
        assert_eq!(reply.usage.completion_tokens, 99);
        assert!(reply.counted);
    }

    /// A stream that never said it finished is a reply that was cut off, and returning it as whole
    /// loses whatever the model was in the middle of writing.
    #[test]
    fn a_reply_is_only_finished_when_the_service_says_so() {
        let mut reply = Reply::default();
        reply.absorb(StreamEvent::ContentBlockDelta {
            index: 0,
            delta: protocol::Delta::Text {
                text: "partial".into(),
            },
        });
        assert!(!reply.ended);

        reply.absorb(StreamEvent::MessageStop {
            stop_reason: Some("end_turn".into()),
        });
        assert!(reply.ended);
    }

    /// Hitting the ceiling is not a transport failure: sending the same request again produces the
    /// same truncation, so it must not be retried and must say what happened.
    #[test]
    fn reaching_the_token_ceiling_is_not_retried() {
        assert!(!worth_another_attempt(1, &BedrockError::TooLong));
        assert!(
            BedrockError::TooLong.to_string().contains("output limit"),
            "{}",
            BedrockError::TooLong
        );
    }

    /// Prompt caching is not something every model this backend can reach offers, and an
    /// inference-profile ARN does not say which model is behind it. A model that refuses the
    /// breakpoints refuses the whole request, so without asking again without them, every request
    /// to such a model fails and the tier is unusable.
    ///
    /// The status is the one measured against a Bedrock-hosted OpenAI model, which answers 403 with
    /// a message about prompt caching rather than the validation status a rejected parameter gets.
    #[test]
    fn a_request_refused_on_its_contents_is_asked_again_without_the_breakpoints() {
        let config = config();
        let egress = Egress::new();
        let mut client = BedrockClient::new(&config, &egress);

        let refused = refusal(403);
        assert!(client.worth_dropping_breakpoints(&refused));

        // Once only. The answer is remembered, so a second refusal is the caller's rather than a
        // request sent for a third time carrying nothing new.
        client.breakpoints = false;
        assert!(!client.worth_dropping_breakpoints(&refused));
    }

    /// The level names a field the model's own provider defines, so a model from another provider
    /// refuses the request on the field name. Measured: a Bedrock-hosted OpenAI model answers 400
    /// `Unknown parameter: 'output_config'`, and answers the same to the OpenAI spelling, so the
    /// level is not this model's to read under any name and the only fix is to stop sending it.
    #[test]
    fn a_request_refused_on_a_parameter_is_asked_again_without_the_effort_level() {
        let config = config();
        let egress = Egress::new();
        let mut client = BedrockClient::new(&config, &egress);

        let refused = refusal(400);
        assert!(client.worth_dropping_effort(&refused));

        client.effort = false;
        assert!(!client.worth_dropping_effort(&refused));
    }

    /// The two are told apart by status alone, so neither concession is spent on the other's
    /// refusal: a rejected parameter must not cost the session its caching, and a model without
    /// caching must not cost it the level it was asked for.
    #[test]
    fn each_refusal_gives_up_only_its_own_part_of_the_request() {
        let config = config();
        let egress = Egress::new();
        let client = BedrockClient::new(&config, &egress);

        assert!(client.worth_dropping_breakpoints(&refusal(403)));
        assert!(!client.worth_dropping_effort(&refusal(403)));

        assert!(client.worth_dropping_effort(&refusal(400)));
        assert!(!client.worth_dropping_breakpoints(&refusal(400)));
    }

    /// A request is refused on its contents for reasons that have nothing to do with the
    /// breakpoints, and asking again without them does not fix one of those. Giving them up anyway
    /// costs full price for a prefix the service would have read once, every round, for the rest of
    /// the session, which is the whole expense the breakpoints exist to avoid.
    #[test]
    fn a_probe_that_failed_as_well_puts_the_breakpoints_back() {
        let config = config();
        let egress = Egress::new();
        let mut client = BedrockClient::new(&config, &egress);

        // What the loop does: drop them, send again, and find the second attempt refused too.
        client.breakpoints = false;
        client.probe_settled(true, true);
        assert!(
            client.breakpoints,
            "a refusal that outlived the breakpoints still cost the session its caching"
        );

        // A probe that answered is the model saying it does not read them, and they stay dropped.
        client.breakpoints = false;
        client.probe_settled(true, false);
        assert!(!client.breakpoints);

        // A request that never probed is left exactly as it was, whichever way it ended.
        for failed in [true, false] {
            client.breakpoints = true;
            client.probe_settled(false, failed);
            assert!(client.breakpoints);
        }
    }

    /// Every other failure leaves both alone. Giving either up on a timeout or a fault would
    /// spend the rest of the session without something nothing had refused.
    #[test]
    fn only_a_refusal_on_the_contents_drops_the_breakpoints() {
        let config = config();
        let egress = Egress::new();
        let client = BedrockClient::new(&config, &egress);

        for status in [402, 404, 429, 500, 503] {
            let error = refusal(status);
            assert!(
                !client.worth_dropping_breakpoints(&error),
                "{status} dropped the breakpoints"
            );
            assert!(
                !client.worth_dropping_effort(&error),
                "{status} dropped the effort level"
            );
        }

        assert!(!client.worth_dropping_breakpoints(&BedrockError::Incomplete));
        assert!(!client.worth_dropping_breakpoints(&BedrockError::TooLong));
    }

    /// A reply the service abandoned says why in the frame that ends it. Reported as a truncation,
    /// the cause is lost and the remedy with it: nothing about "the reply was cut off" tells
    /// somebody their request was refused on its contents.
    #[test]
    fn a_failure_the_service_reported_is_named_rather_than_called_a_truncation() {
        let error = BedrockError::Reported {
            kind: "validationException".to_string(),
        };
        let said = error.to_string();
        assert!(said.contains("validationException"), "{said}");
        assert!(!said.contains("cut off"), "{said}");
    }

    /// A fault or a busy service is worth asking again. A request the service refuses on its
    /// contents is refused identically every time, so asking again spends three round trips to
    /// arrive at the same answer more slowly.
    #[test]
    fn only_the_reported_failures_that_could_pass_are_asked_again() {
        for kind in [
            "throttlingException",
            "modelStreamErrorException",
            "internalServerException",
            "serviceUnavailableException",
        ] {
            let error = BedrockError::Reported {
                kind: kind.to_string(),
            };
            assert!(worth_another_attempt(1, &error), "{kind} was given up on");
        }

        for kind in ["validationException", "somethingNewException"] {
            let error = BedrockError::Reported {
                kind: kind.to_string(),
            };
            assert!(!worth_another_attempt(1, &error), "{kind} was asked again");
        }
    }

    /// A reply that arrived and would not decode is not a connection problem, and an expired
    /// credential is fixed by signing in rather than by asking again.
    #[test]
    fn only_transport_failures_are_retried() {
        assert!(!worth_another_attempt(
            1,
            &BedrockError::Decode {
                detail: "bad".into()
            }
        ));
        assert!(!worth_another_attempt(1, &BedrockError::NoContent));
        assert!(!worth_another_attempt(1, &BedrockError::NoModel));
        assert!(!worth_another_attempt(1, &BedrockError::Cancelled));
        assert!(!worth_another_attempt(
            1,
            &BedrockError::Credentials(credentials::CredentialError::NotInstalled)
        ));

        // A dead connection and lost framing are both worth another attempt.
        assert!(worth_another_attempt(1, &BedrockError::Incomplete));
        assert!(worth_another_attempt(
            1,
            &BedrockError::Frame(eventstream::FrameError::Corrupt { detail: "x".into() })
        ));
    }

    /// Retrying forever turns one failure into a hang. The count is what bounds it.
    #[test]
    fn attempts_are_bounded() {
        assert!(worth_another_attempt(
            ATTEMPTS - 1,
            &BedrockError::Incomplete
        ));
        assert!(!worth_another_attempt(ATTEMPTS, &BedrockError::Incomplete));
    }

    /// Each wait is longer than the last, because the failure this exists for is a network that
    /// needs a moment.
    #[test]
    fn each_backoff_is_longer_than_the_last() {
        assert!(backoff(1) < backoff(2));
        assert!(backoff(2) < backoff(3));
    }
}
