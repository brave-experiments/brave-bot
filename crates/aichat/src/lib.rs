//! Client for the OpenAI-compatible aichat backend.
//!
//! Targets `POST /v1/chat/completions`. Despite the `/v1` prefix that is the **v2**
//! API: the server infers the version from the path, so there is no `/v2/` route to
//! construct. `/v1/conversation` is the older, deprecated surface.
//!
//! Requests are signed rather than bearer-authenticated — see [`bua_signing`]. All
//! traffic goes through [`bua_net::Egress`] so the policy gate sees it, and the model
//! reported in the response is preserved because the server may substitute a different
//! one than was requested.

pub mod protocol;

use bua_config::Config;
use bua_core::event::Sink;
use bua_core::label::Label;
use bua_core::policy::Policy;
use bua_core::value::Labelled;
use bua_net::{Egress, EgressError, Request};
use protocol::{ChatRequest, ChatResponse};
use std::fmt;

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
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(detail) => write!(f, "could not encode the request: {detail}"),
            Self::Decode { detail } => write!(f, "unexpected response: {detail}"),
            Self::Egress(e) => write!(f, "{e}"),
            Self::NoContent => f.write_str("the response contained no message content"),
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
    /// The model reported by the server, which may differ from the one requested —
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

pub struct AichatClient<'a> {
    config: &'a Config,
    egress: &'a Egress,
}

impl<'a> AichatClient<'a> {
    pub fn new(config: &'a Config, egress: &'a Egress) -> Self {
        Self { config, egress }
    }

    /// Send a chat completion request.
    ///
    /// The reply is labelled untrusted-public: it is remote content we do not control,
    /// but carries no confidentiality of ours.
    pub fn complete<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        request: &ChatRequest,
    ) -> Result<Completion, ChatError> {
        let body = serde_json::to_vec(request).map_err(|e| ChatError::Encode(e.to_string()))?;

        let headers =
            bua_signing::sign(self.config.signing_key.expose(), &self.config.key_id, &body);

        let http = Request::post(self.config.chat_completions_url(), body)
            .header("content-type", "application/json")
            .header("digest", &headers.digest)
            .header("authorization", &headers.authorization);

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
}
