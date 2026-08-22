//! What a session remembers between turns.
//!
//! A turn used to begin with nothing but the prompt it was given, so a session was a row of
//! strangers: the model could not be asked to try again, or to carry on, because it had never
//! heard of what came before. This is the record that fixes that.
//!
//! It changes nothing about the rule the repository rests on. Every string in here has already
//! been past [`bua_core::policy::Policy::present`]: either the kernel judged it trusted and
//! showed it to the planner, or what went in is a reference and the content stayed in
//! quarantine. Carrying the record forward therefore carries no untrusted bytes forward, because
//! there were never any in it.
//!
//! Three things travel with the messages:
//!
//! - the **quarantine**, so a reference the planner was given in an earlier turn still names
//!   something. A slot store that died with its turn would leave the conversation full of names
//!   for content that no longer exists.
//! - the **reference counter**, so two turns cannot both hand out `ref:0` and leave the planner
//!   with one name for two things.
//! - the **integrity** the conversation has met, so a later turn cannot label output better than
//!   an earlier turn would have. See [`bua_core::policy::Policy::resuming`].

use bua_aichat::protocol::{Message, Role};
#[cfg(test)]
use bua_aichat::protocol::{ToolCallRequest, ToolCallRequestFunction};
use bua_core::label::Integrity;
use bua_core::slot::{SlotId, SlotStore};
use serde::{Deserialize, Serialize};

/// The record a session carries from one turn to the next.
///
/// Not `Clone`: the quarantine holds the only copy of content nobody may read, and a second
/// store would be a second place for a reference to resolve differently.
#[derive(Debug)]
pub struct Conversation {
    /// The exchange so far, oldest first, without the system prompt.
    ///
    /// The system prompt is left out deliberately: it belongs to the build rather than to the
    /// conversation, so a session that outlives an upgrade should use the new one.
    messages: Vec<Message>,
    /// Content the kernel would not show the planner, by the name it was given.
    quarantine: SlotStore,
    /// How many references the session has handed out.
    ///
    /// Trusted metadata: a counter, never derived from content.
    references: usize,
    /// What everything this conversation has been shown amounts to.
    context: Integrity,
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

impl Conversation {
    /// An empty conversation, which is what a session starts with.
    ///
    /// Trusted, since nothing has been read into it yet.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            quarantine: SlotStore::new(),
            references: 0,
            context: Integrity::Trusted,
        }
    }

    /// Whether anything has been said yet.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// How many messages the next request would carry.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// The whole exchange with the system prompt in front, as one request's messages.
    ///
    /// Any call left unanswered is answered here, saying it did not run. A turn can end between
    /// a call and its result, by cancellation or by failure, and a round that announced a call
    /// nothing ever answered is a malformed request: the next turn would be refused by the
    /// server rather than merely missing something. Filling the gap keeps the record of what
    /// was attempted, which is the reason the conversation survives a failed turn at all.
    pub fn with_system(&self, system: &str) -> Vec<Message> {
        let mut messages = Vec::with_capacity(self.messages.len() + 1);
        messages.push(Message::system(system));

        for (index, message) in self.messages.iter().enumerate() {
            messages.push(message.clone());

            let Some(calls) = &message.tool_calls else {
                continue;
            };
            for call in calls {
                if !self.answered(index, &call.id) {
                    messages.push(Message::tool_result(
                        call.id.clone(),
                        "(this call did not run: the turn ended first)",
                    ));
                }
            }
        }

        messages
    }

    /// Whether the round beginning at `index` answered the call with this id.
    ///
    /// Only the run of results immediately after it counts, since that is where the answers to
    /// a round belong and where a server looks for them.
    fn answered(&self, index: usize, id: &str) -> bool {
        self.messages[index + 1..]
            .iter()
            .take_while(|message| message.tool_call_id.is_some())
            .any(|message| message.tool_call_id.as_deref() == Some(id))
    }

    /// What the conversation has met, for the turn resuming it.
    pub fn context(&self) -> Integrity {
        self.context
    }

    /// Add a message the kernel has already ruled the planner may hold.
    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Take the next reference name.
    ///
    /// Unique across the session rather than across the turn, which is the difference between a
    /// name and a coincidence.
    pub fn next_reference(&mut self) -> SlotId {
        let slot = SlotId::new(format!("ref:{}", self.references));
        self.references += 1;
        slot
    }

    /// The quarantine, for the kernel to write into and read back out of.
    pub fn quarantine(&mut self) -> &mut SlotStore {
        &mut self.quarantine
    }

    /// Record what the turn's context has met.
    ///
    /// Recorded as it happens rather than once the turn is over, because a turn that fails
    /// partway still read what it read, and the next turn has to inherit that.
    ///
    /// One way: [`Integrity::meet`] cannot raise it, so nothing recorded here restores integrity
    /// the conversation has already lost.
    pub fn observed(&mut self, integrity: Integrity) {
        self.context = self.context.meet(integrity);
    }
}

/// One thing said, for an interface showing a conversation it did not watch happen.
///
/// Only the two halves of the exchange a person would recognise. A tool result is left out: it
/// was written for the planner, and a transcript of a resumed session is for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    /// What the user asked.
    User(String),
    /// What the model answered, where the kernel let it be seen at all.
    Assistant(String),
}

/// A conversation written down, for a session that outlives the process.
///
/// Two of the four things a conversation carries, and deliberately so.
///
/// The **messages** are safe to write anywhere: every one of them has already been past the
/// present gate, so a stored conversation holds no untrusted bytes. The **integrity** goes with
/// them because it is what a resumed turn must inherit, and dropping it would let a resumed
/// session call trusted what the original would not have.
///
/// The **quarantine** is not stored. Untrusted content would then be sitting in a file, to be
/// read back and relabelled from what that file says, and a label that survives a round trip
/// through an editable file is not a label. A resumed conversation therefore holds references
/// that no longer name anything, which costs nothing today and is the honest failure: a name
/// with nothing behind it, rather than bytes with a label nobody checked.
///
/// The **reference counter** goes with them so a resumed session cannot hand out a name an
/// earlier message already used.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// The exchange, oldest first, without the system prompt.
    pub messages: Vec<Message>,
    /// What the conversation had met: `trusted`, or anything else.
    ///
    /// A word rather than a flag, so a person reading the file can see what it says, and an
    /// unreadable value means untrusted rather than a parse error. Everything unrecognised
    /// degrades in the safe direction, which is the only direction this may degrade in.
    pub context: String,
    /// How many references had been handed out.
    #[serde(default)]
    pub references: usize,
}

/// The word for an integrity, as it is written down.
const TRUSTED: &str = "trusted";
const UNTRUSTED: &str = "untrusted";

impl Conversation {
    /// The conversation as it can be written down.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            messages: self.messages.clone(),
            context: match self.context {
                Integrity::Trusted => TRUSTED.to_string(),
                Integrity::Untrusted => UNTRUSTED.to_string(),
            },
            references: self.references,
        }
    }

    /// A conversation read back from one.
    ///
    /// The quarantine starts empty, since a snapshot has none: see [`Snapshot`]. Anything but
    /// the word for trusted is read as untrusted, so a truncated, hand-edited or
    /// newer-than-this-build file resumes with less trust rather than more.
    pub fn restored(snapshot: Snapshot) -> Self {
        Self {
            messages: snapshot.messages,
            quarantine: SlotStore::new(),
            references: snapshot.references,
            context: if snapshot.context == TRUSTED {
                Integrity::Trusted
            } else {
                Integrity::Untrusted
            },
        }
    }

    /// The exchange, for an interface that wants to show what was said.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// The exchange as a person would read it back.
    ///
    /// Prompts and answers. A round's own account of itself is left out along with the results,
    /// since between them they are the working, and a transcript being shown to whoever resumed
    /// the session is not the place for it.
    ///
    /// A result sent in the API's own shape is a message of its own and is simply skipped. One
    /// sent as prose, which is what an untrusted context falls back to, is recognised by the
    /// prefix this crate put on it: examining it decides only how a line is drawn, and the text
    /// being examined has already been past the present gate.
    pub fn recounted(&self) -> Vec<Said> {
        self.messages
            .iter()
            .filter_map(|message| match message.role {
                Role::User if message.content.starts_with(TOOL_RESULT_PREFIX) => None,
                Role::User => Some(Said::User(message.content.clone())),
                Role::Assistant if message.tool_calls.is_some() => None,
                Role::Assistant if message.content.trim().is_empty() => None,
                Role::Assistant => Some(Said::Assistant(message.content.clone())),
                Role::System | Role::Tool => None,
            })
            .collect()
    }
}

/// How a tool result is introduced when it is sent as prose rather than in the API's own shape.
///
/// Public because an interface replaying a conversation has to tell one from a prompt, and a
/// literal repeated in two crates is a literal that will disagree with itself.
pub const TOOL_RESULT_PREFIX: &str = "Result of ";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_conversation_has_nothing_in_it_and_has_seen_nothing() {
        let conversation = Conversation::new();
        assert!(conversation.is_empty());
        assert_eq!(conversation.context(), Integrity::Trusted);
    }

    /// The system prompt is the build's, not the conversation's, so it is not stored and not
    /// duplicated when a second turn resumes.
    #[test]
    fn the_system_prompt_is_put_in_front_rather_than_kept() {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("first"));
        conversation.push(Message::assistant("second"));

        let sent = conversation.with_system("be careful");
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[0].content, "be careful");
        assert_eq!(sent[1].content, "first");
        assert_eq!(conversation.len(), 2);

        // And again, with the same result rather than two system prompts.
        assert_eq!(conversation.with_system("be careful").len(), 3);
    }

    /// Two turns handing out the same name would leave the planner with one word for two things,
    /// and the second turn's slot would be refused as a repeated write.
    #[test]
    fn reference_names_are_unique_across_the_session() {
        let mut conversation = Conversation::new();
        let first = conversation.next_reference();
        let second = conversation.next_reference();
        assert_ne!(first, second);
    }

    /// A turn can be cancelled between a call and its result. The round still belongs in the
    /// record, but a call nothing answers makes the whole next request malformed, so the gap is
    /// filled rather than left for a server to refuse.
    #[test]
    fn a_call_nothing_answered_is_answered_before_it_is_sent() {
        let mut conversation = Conversation::new();
        conversation.push(Message::assistant_calling(
            "writing it now",
            vec![ToolCallRequest {
                id: "call-1".to_string(),
                kind: "function".to_string(),
                function: ToolCallRequestFunction {
                    name: "write_file".to_string(),
                    arguments: "{}".to_string(),
                },
            }],
        ));

        let sent = conversation.with_system("be careful");
        let answer = sent.last().expect("a message");
        assert_eq!(answer.tool_call_id.as_deref(), Some("call-1"));
        assert!(answer.content.contains("did not run"));
    }

    /// And a call that was answered is not answered twice, which would read as the tool having
    /// run and then not run.
    #[test]
    fn a_call_that_was_answered_is_left_alone() {
        let mut conversation = Conversation::new();
        conversation.push(Message::assistant_calling(
            "writing it now",
            vec![ToolCallRequest {
                id: "call-1".to_string(),
                kind: "function".to_string(),
                function: ToolCallRequestFunction {
                    name: "write_file".to_string(),
                    arguments: "{}".to_string(),
                },
            }],
        ));
        conversation.push(Message::tool_result("call-1", "wrote index.html"));

        let sent = conversation.with_system("be careful");
        let answers: Vec<_> = sent
            .iter()
            .filter(|m| m.tool_call_id.as_deref() == Some("call-1"))
            .collect();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].content, "wrote index.html");
    }

    /// A session that outlives the process has to come back as what it was, integrity included.
    #[test]
    fn a_conversation_survives_being_written_down() {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("what is 2 + 2?"));
        conversation.push(Message::assistant("four"));
        let _ = conversation.next_reference();

        let restored = Conversation::restored(conversation.snapshot());
        assert_eq!(restored.len(), 2);
        assert_eq!(restored.messages()[0].content, "what is 2 + 2?");
        assert_eq!(restored.context(), Integrity::Trusted);
        // The counter continues rather than starting over, or a resumed session would hand out
        // a name an earlier message already used.
        assert_eq!(
            Conversation::restored(conversation.snapshot()).next_reference(),
            conversation.next_reference()
        );
    }

    /// Integrity is the one thing here that must never come back better than it went in, and a
    /// file is the easiest place to make that mistake.
    #[test]
    fn an_untrusted_conversation_does_not_come_back_trusted() {
        let mut conversation = Conversation::new();
        conversation.observed(Integrity::Untrusted);

        let snapshot = conversation.snapshot();
        assert_eq!(snapshot.context, "untrusted");
        assert_eq!(
            Conversation::restored(snapshot).context(),
            Integrity::Untrusted
        );
    }

    /// Whatever a file says that this build does not recognise, the answer is untrusted. A
    /// truncated write, a hand edit, or a newer build's word all land in the safe direction.
    #[test]
    fn an_unreadable_integrity_is_read_as_untrusted() {
        for word in ["", "TRUSTED", "yes", "somewhat", "trusted-ish"] {
            let restored = Conversation::restored(Snapshot {
                messages: Vec::new(),
                context: word.to_string(),
                references: 0,
            });
            assert_eq!(
                restored.context(),
                Integrity::Untrusted,
                "{word:?} was read as trusted"
            );
        }
    }

    #[test]
    fn what_the_conversation_has_met_only_ever_falls() {
        let mut conversation = Conversation::new();
        conversation.observed(Integrity::Untrusted);
        assert_eq!(conversation.context(), Integrity::Untrusted);

        conversation.observed(Integrity::Trusted);
        assert_eq!(
            conversation.context(),
            Integrity::Untrusted,
            "a later trusted turn does not un-see what an earlier one read"
        );
    }
}
