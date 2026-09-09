//! Answering a question asked beside the work.
//!
//! A person reading a session has questions the session is not about: why a file is shaped the
//! way it is, what a name means, whether an approach was already tried. Asked as a prompt, each
//! one becomes a turn: it joins the conversation, the next turn reads it back, and the planner
//! carries the digression for the rest of the session. Asked here, it does not.
//!
//! **The conversation is read once and never written to.** What leaves it is [`Question`], which
//! is one request's messages: the exchange as the planner would be sent it, with the question on
//! the end. Nothing is pushed, nothing is measured, and no reference is handed out, so the
//! exchange the next turn resumes is the exchange that was there before the question was asked.
//! Taking the request rather than the conversation is what makes that structural: an aside cannot
//! push a message to something it does not have.
//!
//! What makes this sound is the property compaction rests on: every message in a conversation has
//! already been past [`bravebot_core::policy::Policy::present`], so a model given that exchange is
//! given exactly what the planner was given. Nothing new is read and nothing is upgraded.
//!
//! Like a processor, and for the same reasons: no tools in the request, and one round with nothing
//! for a reply to steer.
//!
//! # Two destinations, two gates
//!
//! The answer goes to a person's screen, which is
//! [`bravebot_core::policy::Policy::authorise_display_release`], the same witness a round's own
//! reply takes on its way to the tail of the transcript.
//!
//! It also goes into the session record, so the question and the answer are still there after a
//! resume, and that is a different claim: a record is read back, so what lands in one is what the
//! planner could have held. So the answer goes through `present` as well, and only what came back
//! visible is offered for writing down. Where the exchange has met something untrusted the
//! planner's own words are quarantined like anything else, [`Answered::kept`] is `None`, and the
//! answer is on the screen and nowhere else.

use bravebot_aichat::ChatError;
use bravebot_aichat::protocol::{ChatRequest, Message, Usage};
use bravebot_core::event::Sink;
use bravebot_core::label::Integrity;
use bravebot_core::policy::{Denial, Policy};
use bravebot_core::reference::Presentation;
use bravebot_core::slot::SlotId;
use std::fmt;

use crate::conversation::Conversation;
use crate::processor::Chat;

/// What the model is told it is doing.
///
/// Addressed to a model that is about to read a whole working session and then a question that is
/// not part of it. Without this the exchange above reads as the instruction and the question reads
/// as the next step of the work, which is the one thing an aside must not become.
const SYSTEM_PROMPT: &str = "\
A person and a coding agent have been working together, and their exchange is shown to you below. \
You are that agent, and the person has stopped to ask you something beside the work.

The last message is their question. Answer it and do nothing else. Do not carry the work on, do \
not propose a next step, and do not act as though the question were an instruction: it is a \
question, even where it is phrased as one about what to do.

You have no tools this time, so answer from the exchange in front of you and from what you know. \
Where answering properly would mean reading a file or running something, say what you would look \
at and why, and give the best answer you can without it. Do not invent the contents of anything \
you have not been shown.

Your answer is shown to the person and then set aside. It does not go back into the exchange \
above, so the agent picking the work up will not have read it: nothing you write here is a \
decision, and the person will have to say it again themselves if they want it acted on.

Be brief. Answer the question first, in as few words as it takes, and add the reasoning after it \
only where the answer is no use without it.";

/// What introduces the person's own question.
///
/// The driver's words, as trusted as the system prompt beside them. A conversation ends with
/// somebody having said something, so a bare question on the end is a continuation of it; this is
/// what makes the last message read as the thing being asked.
const INSTRUCTION: &str = "Setting the work aside for a moment, here is my question:";

/// One question, and the exchange it was asked beside.
///
/// Taken off the conversation before anything else happens, on the thread that holds it, because
/// a [`Conversation`] is deliberately not `Clone`: its quarantine holds the only copy of content
/// nobody may read, and a second store would be a second place for a reference to resolve
/// differently. So this is what crosses to a worker thread, and it is the narrower thing to hand
/// over anyway.
#[derive(Debug, Clone)]
pub struct Question {
    /// The request to send: the exchange with the question's own message on the end.
    messages: Vec<Message>,
    /// What the exchange had met, which is what decides whether the answer may be written down.
    context: Integrity,
}

impl Question {
    /// Fork the exchange and put the question on the end of it.
    ///
    /// `question` is the line the person typed, which is trusted in the sense a prompt is: it came
    /// from the keyboard of the user who owns the session.
    pub fn about(conversation: &Conversation, question: &str) -> Self {
        let mut messages = conversation.with_system(SYSTEM_PROMPT);
        messages.push(Message::user(format!("{INSTRUCTION}\n\n{question}")));
        Self {
            messages,
            context: conversation.context(),
        }
    }

    /// What the exchange had met when the question was asked.
    pub fn context(&self) -> Integrity {
        self.context
    }
}

/// What one aside produced.
pub struct Answered {
    /// The answer, released for the person's screen.
    pub shown: String,
    /// The same answer, where the planner could have held it.
    ///
    /// `None` where it could not, which is a conversation that has met something untrusted. The
    /// person still reads it; the record does not keep it, because a record is read back.
    pub kept: Option<String>,
    /// What the question cost, so the session can charge it.
    pub usage: Usage,
}

#[derive(Debug)]
pub enum AsideError {
    /// The answer was refused on its way past a gate.
    Denied(Denial),
    /// The call failed or was refused in transit.
    Chat(crate::backend::BackendError),
}

impl fmt::Display for AsideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied(d) => write!(f, "{d}"),
            Self::Chat(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AsideError {}

impl From<Denial> for AsideError {
    fn from(value: Denial) -> Self {
        Self::Denied(value)
    }
}

impl From<crate::backend::BackendError> for AsideError {
    fn from(value: crate::backend::BackendError) -> Self {
        Self::Chat(value)
    }
}

impl From<ChatError> for AsideError {
    fn from(value: ChatError) -> Self {
        Self::Chat(value.into())
    }
}

/// Ask one question beside the work, and hand back what came of it.
///
/// `watching` is given the answer as it arrives, so a person waiting on one sees it being
/// written. It is handed released text, because whether there is anything worth drawing is a
/// question about the words and the interface is the side allowed to ask that one.
pub fn ask<S: Sink>(
    policy: &mut Policy<'_, S>,
    chat: &mut Chat<'_>,
    question: Question,
    mut watching: impl FnMut(&str),
) -> Result<Answered, AsideError> {
    // No tools, deliberately and visibly: `ChatRequest::new` leaves the field empty and nothing
    // below adds to it. A question that could call a tool would be a turn, and a turn is the
    // thing this exists to avoid being.
    let model = chat.model.unwrap_or(&chat.config.default_model);
    let request = ChatRequest::new(model, question.messages);

    let mut client = crate::backend::Backend::select(chat.config, chat.egress, model);
    if let Some(cancel) = chat.cancel {
        client = client.with_cancel(cancel.clone());
    }
    if let Some(subscription) = chat.subscription.as_deref_mut() {
        client = client.with_subscription(subscription);
    }

    // Minted before the request goes out, because the gate needs the policy and the policy is
    // lent to the client for the duration of the call. One witness for the whole answer rather
    // than one per frame: the release is the same release however many chunks it arrives in.
    let as_written =
        policy.authorise_display_release("an answer to a question asked beside the work");
    let completion = client.complete_streaming(policy, &request, |progress| {
        watching(progress.written.declassify(&as_written));
    })?;

    // Relabelled from the context the way a round's own words are: what comes back from the
    // client carries the label the network gave it, and the kernel is the only thing that knows
    // what this model was shown.
    let written = {
        let (text, _) = completion.content.clone().into_parts_for_decoding();
        policy.label_model_output("btw", text)
    };

    // The record's gate: it says whether the planner could have held these words, which is what
    // decides whether they may be written down. A store of its own, thrown away with the request,
    // because a name out of the conversation's counter would be a change to the exchange and
    // nothing ever resolves this reference: a quarantined answer is not written down at all.
    let mut aside_only = bravebot_core::slot::SlotStore::new();
    let kept = match policy.present(
        "btw",
        SlotId::new("aside"),
        "the answer to a question asked beside the work",
        &written,
        &mut aside_only,
    )? {
        Presentation::Visible(body) => Some(body),
        Presentation::Quarantined(_) => None,
    };

    let shown = completion.content.declassify(&as_written);

    Ok(Answered {
        shown,
        kept,
        usage: completion.usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn an_exchange() -> Conversation {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("add the feature"));
        conversation.push(Message::assistant("done"));
        conversation
    }

    /// The exchange reads as the instruction unless something says otherwise, and a question on
    /// the end of one reads as the next step of the work. A model that took it that way would
    /// answer by proposing an edit, which is what this whole path exists to avoid.
    #[test]
    fn the_question_is_marked_as_one_rather_than_left_on_the_end_of_the_work() {
        let asking = Question::about(&an_exchange(), "why is the parser recursive?");

        let last = asking
            .messages
            .last()
            .expect("the question closes the request");
        let text = last.content.text();
        assert!(
            text.starts_with(INSTRUCTION),
            "the question was sent without anything saying it was one: {text}"
        );
        assert!(
            text.contains("why is the parser recursive?"),
            "the question itself did not reach the request: {text}"
        );
    }

    /// The whole exchange goes out, because a question about the work cannot be answered from the
    /// question alone.
    #[test]
    fn the_exchange_goes_out_with_the_question() {
        let asking = Question::about(&an_exchange(), "why is the parser recursive?");

        let said: Vec<String> = asking
            .messages
            .iter()
            .map(|message| message.content.text())
            .collect();
        assert!(
            said.iter().any(|text| text.contains("add the feature")),
            "the exchange was not sent with the question: {said:?}"
        );
        assert!(
            said.first().is_some_and(|text| text.contains("no tools")),
            "the system prompt did not lead the request: {said:?}"
        );
    }

    /// The whole point is that the exchange the next turn resumes is untouched. A question that
    /// grew the conversation would be a turn wearing a different name.
    #[test]
    fn asking_leaves_the_exchange_the_length_it_was() {
        let conversation = an_exchange();
        let before = conversation.len();

        let _ = Question::about(&conversation, "why is the parser recursive?");

        assert_eq!(
            conversation.len(),
            before,
            "asking a question changed the conversation"
        );
    }

    /// The label the answer is judged against is the one the exchange had when the question was
    /// asked. A question taken over an untrusted exchange whose context arrived as trusted would
    /// have its answer written down, which is the laundering route the record's rule closes.
    #[test]
    fn the_question_carries_what_the_exchange_had_met() {
        let mut conversation = an_exchange();
        assert_eq!(
            Question::about(&conversation, "why?").context(),
            Integrity::Trusted
        );

        conversation.observed(Integrity::Untrusted);
        assert_eq!(
            Question::about(&conversation, "why?").context(),
            Integrity::Untrusted
        );
    }

    /// A model told to answer and nothing else still has to be told what it may not reach for,
    /// because the exchange above it is full of tool calls that worked.
    #[test]
    fn the_model_is_told_it_has_no_tools_and_must_not_invent_what_it_cannot_read() {
        assert!(
            SYSTEM_PROMPT.contains("You have no tools this time"),
            "nothing told the model the tools in the exchange above are not available to it"
        );
        assert!(
            SYSTEM_PROMPT.contains("Do not invent the contents"),
            "a model with no tools and no such instruction answers from an imagined file"
        );
    }
}
