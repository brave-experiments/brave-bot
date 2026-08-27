//! Shortening a conversation into a summary of itself.
//!
//! A session's request grows with every round, since each one re-sends the whole history, and
//! nothing ever gave any of it back. Compaction is what does: the older part of the exchange
//! stops being sent and a summary of it is sent instead. What the person sees is untouched, and
//! so is what the quarantine holds; see [`crate::conversation::Conversation::compacted`].
//!
//! **This is not a processor, and must not become one.** A processor is the one component allowed
//! to read untrusted content, and the price of that is that everything it produces is quarantined:
//! the planner gets a reference and never the bytes. A summary the planner may not read is not a
//! summary it can carry on from, so routing compaction through one would produce a feature that
//! cannot do the only thing it is for.
//!
//! What makes this call sound is the opposite property. Every message in a conversation has
//! already been past [`bravebot_core::policy::Policy::present`]: either the kernel judged it trusted
//! and showed it to the planner, or what went in was a reference and the bytes stayed in
//! quarantine. So a model given that exchange is given exactly what the planner was given, and
//! [`bravebot_core::policy::Policy::label_model_output`] labels what it writes the way it labels
//! anything else the planner said. Nothing is upgraded and nothing new is read.
//!
//! [`bravebot_core::policy::Policy::adopt_summary`] is the gate on the way back in, and it refuses once
//! the conversation's integrity has fallen. A refusal leaves the conversation exactly as it was.
//!
//! Like a processor, and for the same reasons: no tools in the request, and one round with nothing
//! for a reply to steer.

use bravebot_aichat::protocol::{ChatRequest, Message, Usage};
use bravebot_aichat::{AichatClient, ChatError};
use bravebot_core::event::Sink;
use bravebot_core::policy::{Denial, Policy};
use std::fmt;

use crate::conversation::Conversation;
use crate::processor::Chat;

/// What the summariser is told to produce.
///
/// Addressed to the agent that will read it rather than to the person, because the person keeps
/// the transcript either way and it is the agent that has to pick the work up mid-sentence.
const SYSTEM_PROMPT: &str = "\
You are summarising part of a conversation between a person and a coding agent, so that the agent \
can carry on with less of it in front of it. What you write replaces that part of the exchange \
entirely: it is the only thing that will remain of it, so anything you leave out is gone.

The most recent exchanges are not shown to you. They are kept exactly as they are, so do not try \
to account for them and do not write a conclusion.

Keep, in whatever order reads best:

- what the person asked for, in their own words wherever the words matter
- what was decided, and what was decided against, and why
- every path read, written or created, spelled exactly as it appeared
- commands that were run, and what came of them
- what is finished, what is half done, and what has not been started
- every ref:N that was mentioned, and what each one was about
- anything the person corrected the agent about
- questions still outstanding

Leave out the agent's account of how it got somewhere, and anything later work superseded.

Write plain prose, or prose with a short list in it. No preamble, no sign-off, and nothing about \
the fact that you are summarising: begin with the work itself.";

/// The instruction that closes the request.
///
/// A conversation ends with someone having said something, so without this the model is being
/// asked to continue it rather than to summarise it. The driver's own words, as trusted as the
/// system prompt beside them.
const INSTRUCTION: &str = "Summarise everything above, as your instructions describe.";

/// What one compaction did.
pub struct Compacted {
    /// How many messages stopped being sent.
    pub summarised: usize,
    /// How many are still sent word for word, the summary not counted.
    pub kept: usize,
    /// The model the server reported using, which may differ from the one asked for.
    pub model: String,
    /// What the summary cost, so a turn can report the whole of what it spent.
    pub usage: Usage,
}

#[derive(Debug)]
pub enum CompactError {
    /// The summary was refused on the way back into the context.
    Denied(Denial),
    /// The call failed or was refused in transit.
    Chat(ChatError),
}

impl fmt::Display for CompactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied(d) => write!(f, "{d}"),
            Self::Chat(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CompactError {}

impl From<Denial> for CompactError {
    fn from(value: Denial) -> Self {
        Self::Denied(value)
    }
}

impl From<ChatError> for CompactError {
    fn from(value: ChatError) -> Self {
        Self::Chat(value)
    }
}

/// Compact a conversation, where there is anything worth compacting.
///
/// `Ok(None)` means there was not: a conversation that is all recent, or one whose older part is
/// already nothing but an earlier summary. Not an error, since asking is how a caller finds out,
/// and there is nothing wrong with the answer being no.
///
/// Nothing happens to the conversation unless the whole of this succeeds. A refused summary, a
/// failed request or a cancelled turn leaves a session with the history it already had, which is
/// longer than anyone wanted but is the one thing here that is never wrong.
pub fn compact<S: Sink>(
    policy: &mut Policy<'_, S>,
    chat: &mut Chat<'_>,
    conversation: &mut Conversation,
) -> Result<Option<Compacted>, CompactError> {
    let Some(boundary) = conversation.compaction_boundary() else {
        return Ok(None);
    };

    let mut messages = conversation.to_summarise(boundary, SYSTEM_PROMPT);
    messages.push(Message::user(INSTRUCTION));

    // No tools, deliberately and visibly: `ChatRequest::new` leaves the field empty and nothing
    // below adds to it. A summariser with a tool would be a second planner, and a second planner
    // is a second thing to reason about rather than a shorter conversation.
    let model = chat.model.unwrap_or(&chat.config.default_model);
    let request = ChatRequest::new(model, messages);

    let mut client = AichatClient::new(chat.config, chat.egress);
    if let Some(subscription) = chat.subscription.as_deref_mut() {
        client = client.with_subscription(subscription);
    }

    // Streamed because that is the shape the backend answers in. Nothing watches the pieces go
    // by: a summary appearing a word at a time in place of the transcript it replaces would read
    // as the agent saying it, and it is not saying it to anyone.
    let completion = client.complete_streaming(policy, &request, |_| {})?;

    // Relabelled from the context the way a round's own words are, and for the same reason: what
    // comes back from the client carries the label the network gave it, and the kernel is the
    // only thing that knows what this model was shown.
    let written = {
        let (text, _) = completion.content.into_parts_for_decoding();
        policy.label_model_output("compact", text)
    };
    let summary = policy.adopt_summary(&written)?;
    let kept = conversation.len() - boundary;
    conversation.compacted(boundary, &summary);

    Ok(Some(Compacted {
        summarised: boundary,
        kept,
        model: completion.model,
        usage: completion.usage,
    }))
}
