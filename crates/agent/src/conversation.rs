//! What a session remembers between turns.
//!
//! A turn used to begin with nothing but the prompt it was given, so a session was a row of
//! strangers: the model could not be asked to try again, or to carry on, because it had never
//! heard of what came before. This is the record that fixes that.
//!
//! It changes nothing about the rule the repository rests on. Every string in here has already
//! been past [`bravebot_core::policy::Policy::present`]: either the kernel judged it trusted and
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
//!   an earlier turn would have. See [`bravebot_core::policy::Policy::resuming`].

use bravebot_aichat::protocol::{Message, Role};
#[cfg(test)]
use bravebot_aichat::protocol::{ToolCallRequest, ToolCallRequestFunction};
use bravebot_core::label::Integrity;
use bravebot_core::slot::{SlotId, SlotStore};
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
    /// What compaction took out of the request, oldest first.
    ///
    /// What compaction shortens is the request, not the record. These messages are no longer
    /// sent, and a summary of them is sent instead, but the person whose session this is still
    /// owns every word of it: [`Conversation::recounted`] reads them back. A transcript with a
    /// hole in it where the user's own earlier prompts were is not a saving worth making.
    archive: Vec<Message>,
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
            archive: Vec::new(),
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
        Self::assembled(system, &self.messages)
    }

    /// The same, over some prefix of the exchange.
    ///
    /// Shared with [`Conversation::with_system`] rather than written twice, because the gap
    /// filling below is the difference between a well-formed request and one a server refuses,
    /// and a summariser sending a prefix needs it exactly as much as a turn sending the whole.
    fn assembled(system: &str, exchange: &[Message]) -> Vec<Message> {
        let mut messages = Vec::with_capacity(exchange.len() + 1);
        messages.push(Message::system(system));

        for (index, message) in exchange.iter().enumerate() {
            messages.push(message.clone());

            let Some(calls) = &message.tool_calls else {
                continue;
            };
            for call in calls {
                if !answered(exchange, index, &call.id) {
                    messages.push(Message::tool_result(
                        call.id.clone(),
                        "(this call did not run: the turn ended first)",
                    ));
                }
            }
        }

        messages
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

    /// Where compaction would cut, or `None` when there is nothing worth summarising.
    ///
    /// The index is always the start of an exchange, never the middle of one. That is not a
    /// preference: [`Conversation::with_system`] answers a call nothing answered, and it looks
    /// only at the run of results immediately after the call, so an assistant message separated
    /// from its results would leave both halves malformed. The boundaries are found from roles
    /// and call ids, which are the shape of the exchange rather than anything anyone wrote.
    ///
    /// [`RECENT_EXCHANGES_KEPT`] of them stay word for word. A summary is lossy, and what a
    /// session is in the middle of is the part that can least afford to be paraphrased.
    ///
    /// `None` where the head would be nothing but an earlier summary. There is nothing left to
    /// give up in that case, and saying so is better than spending a request to learn it.
    pub fn compaction_boundary(&self) -> Option<usize> {
        let opens: Vec<usize> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, message)| opens_an_exchange(message))
            .map(|(index, _)| index)
            .collect();

        let cut = *opens.get(opens.len().checked_sub(RECENT_EXCHANGES_KEPT)?)?;
        self.messages[..cut]
            .iter()
            .any(|message| {
                !message
                    .content
                    .as_text()
                    .is_some_and(|text| text.starts_with(COMPACTED_PREFIX))
            })
            .then_some(cut)
    }

    /// The part compaction would replace, as one request's messages.
    ///
    /// The system prompt is the summariser's, so it is passed in the same way a turn passes its
    /// own: the conversation stores neither.
    pub fn to_summarise(&self, boundary: usize, system: &str) -> Vec<Message> {
        Self::assembled(system, &self.messages[..boundary])
    }

    /// Put a summary in place of everything before `boundary`.
    ///
    /// Three things it deliberately leaves alone. The **quarantine**, because it holds the only
    /// copy of the content the surviving references name, and a reference in a retained message
    /// has to still resolve. The **reference counter**, because slots are written once and a name
    /// handed out twice is a collision. The **integrity**, because compaction is not a fresh
    /// session and nothing here has un-read what the conversation read.
    ///
    /// The replaced messages go to the archive rather than into a bin. See the field.
    pub fn compacted(&mut self, boundary: usize, summary: &str) {
        let replaced: Vec<Message> = self.messages.drain(..boundary).collect();
        self.archive.extend(replaced);

        let mut note = format!("{COMPACTED_PREFIX}\n\n{}", summary.trim());
        if let Some(live) = self.live_references() {
            note.push_str("\n\n");
            note.push_str(&live);
        }
        self.messages.insert(0, Message::user(note));
    }

    /// What to tell the planner about the references it may still use.
    ///
    /// The names are still in the summary and in the retained messages, and nothing in either
    /// says whether they survived, so the planner would find out by being refused. Written from
    /// the counter and the slot store's inventory of ids: no origin, and no byte of anything
    /// quarantined. An origin out of a quarantined listing is content, and this line is going
    /// into the planner's context.
    fn live_references(&self) -> Option<String> {
        let names: Vec<String> = (0..self.references)
            .map(|n| SlotId::new(format!("ref:{n}")))
            .filter(|slot| self.quarantine.label_of(slot).is_some())
            .map(|slot| slot.to_string())
            .collect();

        (!names.is_empty()).then(|| {
            format!(
                "The conversation above was summarised, but nothing was thrown away behind it: \
                 {} still name what they named.",
                names.join(", ")
            )
        })
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
/// The exchange a person would recognise, and what the turn did between the two. A tool
/// *result* is left out: it was written for the planner, and a transcript of a resumed session
/// is for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    /// What the user asked.
    User(String),
    /// What the model answered, where the kernel let it be seen at all.
    Assistant(String),
    /// A call the turn made, in the words it was announced with while it ran.
    ///
    /// What came of it is not here. The record does not say, and inventing an outcome for a call
    /// whose result nobody wrote down would be worse than admitting the line is all there is.
    Tool(String),
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
/// that no longer name anything, which is the honest failure: a name with nothing behind it,
/// rather than bytes with a label nobody checked. [`Conversation::restored`] says so rather than
/// leaving the planner to find out by being refused.
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
    /// What compaction took out of the request, oldest first.
    ///
    /// Written down because it is the person's own transcript and losing it on a resume would
    /// make compaction cost them their history after all. Defaulted, so a session file written
    /// before compaction existed still reads.
    #[serde(default)]
    pub archive: Vec<Message>,
}

/// The word for an integrity, as it is written down.
const TRUSTED: &str = "trusted";
const UNTRUSTED: &str = "untrusted";

impl Conversation {
    /// The conversation as it can be written down.
    ///
    /// The note [`Conversation::restored`] adds is left out. It is something a resume produces
    /// rather than part of the exchange, and writing it down would stack another copy on every
    /// resume while the one already there named a shorter list than the session had by then.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            messages: self
                .messages
                .iter()
                .filter(|message| {
                    !(message.role == Role::User
                        && message
                            .content
                            .as_text()
                            .is_some_and(|text| text.starts_with(RESUMED_PREFIX)))
                })
                .cloned()
                .collect(),
            context: match self.context {
                Integrity::Trusted => TRUSTED.to_string(),
                Integrity::Untrusted => UNTRUSTED.to_string(),
            },
            references: self.references,
            archive: self.archive.clone(),
        }
    }

    /// A conversation read back from one.
    ///
    /// The quarantine starts empty, since a snapshot has none: see [`Snapshot`]. Anything but
    /// the word for trusted is read as untrusted, so a truncated, hand-edited or
    /// newer-than-this-build file resumes with less trust rather than more.
    ///
    /// A conversation that had been handed references is told they are dead. The names are still
    /// in the transcript in front of the planner, and nothing in it says they stopped naming
    /// anything, so without this the first thing a resumed session does with quarantined content
    /// is spend a call finding out. Saying so is cheap and needs no content: the note is written
    /// here, from a counter the kernel kept, and holds no byte of what was quarantined.
    pub fn restored(snapshot: Snapshot) -> Self {
        let mut messages = snapshot.messages;
        if let Some(note) = dead_references(snapshot.references) {
            messages.push(Message::user(note));
        }

        Self {
            messages,
            quarantine: SlotStore::new(),
            references: snapshot.references,
            context: if snapshot.context == TRUSTED {
                Integrity::Trusted
            } else {
                Integrity::Untrusted
            },
            archive: snapshot.archive,
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
    ///
    /// The **calls** are reported as well, since what a turn did is most of what a person wants
    /// back. Only that it happened and what it was about: a call's result is not recounted, so
    /// resuming a session does not put a file's contents on the screen where the live session
    /// showed a one-line summary.
    pub fn recounted(&self) -> Vec<Said> {
        let mut said = Vec::new();
        // The archive first: what compaction took out of the request is still the person's
        // session, and they are the one reading this.
        for message in self.archive.iter().chain(self.messages.iter()) {
            match message.role {
                Role::User
                    if message
                        .content
                        .as_text()
                        .is_some_and(|text| text.starts_with(TOOL_RESULT_PREFIX)) => {}
                // Addressed to the planner, not said by anyone. Drawn as a prompt it would look
                // like something the user typed and never did.
                Role::User
                    if message
                        .content
                        .as_text()
                        .is_some_and(|text| text.starts_with(RESUMED_PREFIX)) => {}
                // Written for the planner, and the archive above already holds what it stands
                // in for, so drawing it would show the session twice.
                Role::User
                    if message
                        .content
                        .as_text()
                        .is_some_and(|text| text.starts_with(COMPACTED_PREFIX)) => {}
                // `text` rather than the whole content: an attachment is drawn from what the
                // interface recorded about it, and a data URI in the scrollback is not a transcript.
                Role::User => said.push(Said::User(message.content.text())),
                Role::Assistant => {
                    // What the model said on its way to a call, which the live transcript shows
                    // above the call it introduces.
                    let spoken = message.content.text();
                    if !spoken.trim().is_empty() {
                        said.push(Said::Assistant(spoken));
                    }
                    for call in message.tool_calls.iter().flatten() {
                        said.push(Said::Tool(crate::tools::describe_stored_call(
                            &call.function.name,
                            &call.function.arguments,
                        )));
                    }
                }
                Role::System | Role::Tool => {}
            }
        }
        said
    }
}

/// Whether the round beginning at `index` answered the call with this id.
///
/// Only the run of results immediately after it counts, since that is where the answers to a
/// round belong and where a server looks for them.
fn answered(exchange: &[Message], index: usize, id: &str) -> bool {
    exchange[index + 1..]
        .iter()
        .take_while(|message| message.tool_call_id.is_some())
        .any(|message| message.tool_call_id.as_deref() == Some(id))
}

/// How many of the most recent exchanges compaction leaves word for word.
///
/// Two rather than one, because the exchange a session is in the middle of usually refers to the
/// one before it: "do the same to the other file" is unanswerable from a paraphrase of what the
/// other file was.
const RECENT_EXCHANGES_KEPT: usize = 2;

/// Whether a message begins an exchange, so compaction may cut in front of it.
///
/// A prompt the user typed, or a summary standing in for the ones before it. A tool result sent
/// as prose is not one, whatever it looks like: it belongs to the round above it, and cutting
/// between the two would separate a call from its answer.
fn opens_an_exchange(message: &Message) -> bool {
    message.role == Role::User
        && !message
            .content
            .as_text()
            .is_some_and(|text| text.starts_with(TOOL_RESULT_PREFIX))
        && !message
            .content
            .as_text()
            .is_some_and(|text| text.starts_with(RESUMED_PREFIX))
}

/// How a tool result is introduced when it is sent as prose rather than in the API's own shape.
///
/// Public because an interface replaying a conversation has to tell one from a prompt, and a
/// literal repeated in two crates is a literal that will disagree with itself.
pub const TOOL_RESULT_PREFIX: &str = "Result of ";

/// How the note about a resume begins, so a transcript can tell it from something a person said.
pub const RESUMED_PREFIX: &str = "This session was resumed.";

/// How the summary standing in for a compacted exchange begins.
///
/// Public for the same reason as the others: a transcript has to tell it from a prompt, and a
/// literal repeated in two crates is a literal that will disagree with itself.
pub const COMPACTED_PREFIX: &str = "Earlier in this conversation:";

/// What to tell the planner about the references it was handed before the resume.
///
/// `None` when the session never quarantined anything, since a note about references nobody was
/// given is noise in the context of every resumed session that never read an untrusted file.
///
/// The counter is the whole of the input, and a counter is not content: it says how many names
/// were handed out, never what was behind any of them.
fn dead_references(references: usize) -> Option<String> {
    let names = match references {
        0 => return None,
        1 => "ref:0".to_string(),
        n => format!("ref:0 to ref:{}", n - 1),
    };
    Some(format!(
        "{RESUMED_PREFIX} The quarantined content behind {names} was not kept, so those \
         references no longer name anything and using one will be refused. Read a file again to \
         be given a fresh reference to it."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_core::label::Label;

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
        assert_eq!(sent[0].content.text(), "be careful");
        assert_eq!(sent[1].content.text(), "first");
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

    fn a_call(name: &str, arguments: &str) -> ToolCallRequest {
        ToolCallRequest {
            id: "call-1".to_string(),
            kind: "function".to_string(),
            function: ToolCallRequestFunction {
                name: name.to_string(),
                arguments: arguments.to_string(),
            },
        }
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
        assert!(answer.content.text().contains("did not run"));
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
        assert_eq!(answers[0].content.text(), "wrote index.html");
    }

    /// A session that outlives the process has to come back as what it was, integrity included.
    #[test]
    fn a_conversation_survives_being_written_down() {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("what is 2 + 2?"));
        conversation.push(Message::assistant("four"));
        let _ = conversation.next_reference();

        let restored = Conversation::restored(conversation.snapshot());
        assert_eq!(restored.messages()[0].content.text(), "what is 2 + 2?");
        assert_eq!(restored.messages()[1].content.text(), "four");
        assert_eq!(restored.context(), Integrity::Trusted);
        // The counter continues rather than starting over, or a resumed session would hand out
        // a name an earlier message already used.
        assert_eq!(
            Conversation::restored(conversation.snapshot()).next_reference(),
            conversation.next_reference()
        );
    }

    /// The quarantine does not survive, so every reference the planner holds is a name for
    /// nothing. Nothing in the transcript says so, which is what left a resumed session to find
    /// out by asking for content that no longer exists.
    #[test]
    fn a_resumed_conversation_is_told_its_references_are_dead() {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("summarise notes.md"));
        let _ = conversation.next_reference();
        let _ = conversation.next_reference();
        let _ = conversation.next_reference();

        let restored = Conversation::restored(conversation.snapshot());
        let note = restored.messages().last().expect("a note").content.text();
        assert!(note.contains("ref:0"), "{note}");
        assert!(note.contains("ref:2"), "{note}");
        assert!(
            !note.contains("ref:3"),
            "a name that was never handed out: {note}"
        );
    }

    /// One reference is named rather than described as a range, since "ref:0 to ref:0" is a way
    /// of writing one name that invites a reader to look for two.
    #[test]
    fn a_single_dead_reference_is_named_on_its_own() {
        let mut conversation = Conversation::new();
        let _ = conversation.next_reference();

        let restored = Conversation::restored(conversation.snapshot());
        let note = restored.messages().last().expect("a note").content.text();
        assert!(note.contains("ref:0"), "{note}");
        assert_eq!(
            note.matches("ref:").count(),
            1,
            "one reference was described as a range: {note}"
        );
    }

    /// A session that never quarantined anything gets no note. It would be in the context of
    /// every resumed session, saying nothing about anything.
    #[test]
    fn a_conversation_that_was_handed_no_references_is_told_nothing() {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("what is 2 + 2?"));
        conversation.push(Message::assistant("four"));

        let restored = Conversation::restored(conversation.snapshot());
        assert_eq!(restored.len(), 2, "a note appeared with nothing to say");
    }

    /// What a turn did is most of what a person resumes a session to see. Prose alone left a
    /// transcript that said the model answered and never said it had read anything.
    #[test]
    fn a_recounted_turn_says_what_it_did_and_not_only_what_it_said() {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("what is in main.rs?"));
        conversation.push(Message::assistant_calling(
            "let me look",
            vec![a_call("read_file", r#"{"path":"src/main.rs"}"#)],
        ));
        conversation.push(Message::tool_result("call-1", "fn main() {}"));
        conversation.push(Message::assistant("it is a hello world"));

        assert_eq!(
            conversation.recounted(),
            vec![
                Said::User("what is in main.rs?".to_string()),
                Said::Assistant("let me look".to_string()),
                Said::Tool("Read(src/main.rs)".to_string()),
                Said::Assistant("it is a hello world".to_string()),
            ]
        );
    }

    /// The result stays out. A live session showed a one-line summary beside the call, and
    /// putting the file's contents there instead would be a resume showing more than the session
    /// it is resuming ever did.
    #[test]
    fn what_a_call_returned_is_not_recounted() {
        let mut conversation = Conversation::new();
        conversation.push(Message::assistant_calling(
            String::new(),
            vec![a_call("read_file", r#"{"path":"secrets.txt"}"#)],
        ));
        conversation.push(Message::tool_result("call-1", "the file's whole contents"));
        // A result sent as prose, which is the fallback in an untrusted context.
        conversation.push(Message::user(format!(
            "{TOOL_RESULT_PREFIX}read_file: the file's whole contents"
        )));

        let recounted = conversation.recounted();
        assert_eq!(recounted, vec![Said::Tool("Read(secrets.txt)".to_string())]);
    }

    /// A round with several calls is several lines, in the order they were asked for.
    #[test]
    fn every_call_in_a_round_is_recounted() {
        let mut conversation = Conversation::new();
        conversation.push(Message::assistant_calling(
            String::new(),
            vec![
                a_call("search", r#"{"pattern":"MAX_STEPS"}"#),
                a_call("list_files", r#"{"directory":"src"}"#),
            ],
        ));

        assert_eq!(
            conversation.recounted(),
            vec![
                Said::Tool("Search(MAX_STEPS)".to_string()),
                Said::Tool("List(src)".to_string()),
            ]
        );
    }

    /// Arguments a turn ended before writing must not take the line with them. A call announced
    /// and never completed is exactly what a killed session leaves behind.
    #[test]
    fn a_call_with_unreadable_arguments_is_still_recounted() {
        let mut conversation = Conversation::new();
        conversation.push(Message::assistant_calling(
            String::new(),
            vec![a_call("read_file", "{\"path\":")],
        ));

        assert_eq!(
            conversation.recounted(),
            vec![Said::Tool("Read".to_string())]
        );
    }

    /// The note is for the planner. Drawn in a transcript it would read as a prompt the user
    /// never typed, in a session they are resuming precisely to see what was said.
    #[test]
    fn the_note_is_not_shown_as_something_the_user_said() {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("summarise notes.md"));
        let _ = conversation.next_reference();

        let restored = Conversation::restored(conversation.snapshot());
        assert_eq!(
            restored.recounted(),
            vec![Said::User("summarise notes.md".to_string())]
        );
    }

    /// Restoring twice must not stack notes, which is what would happen if the note were saved
    /// with the messages and then added again on the way back in.
    #[test]
    fn resuming_a_resumed_session_does_not_repeat_the_note() {
        let mut conversation = Conversation::new();
        let _ = conversation.next_reference();

        let once = Conversation::restored(conversation.snapshot());
        let twice = Conversation::restored(once.snapshot());
        let notes = twice
            .messages()
            .iter()
            .filter(|message| {
                message
                    .content
                    .as_text()
                    .is_some_and(|text| text.starts_with(RESUMED_PREFIX))
            })
            .count();
        assert_eq!(notes, 1, "the note was added again on top of itself");
    }

    /// Three exchanges, the shape most of the compaction tests need.
    fn three_exchanges() -> Conversation {
        let mut conversation = Conversation::new();
        for (prompt, answer) in [("first", "a"), ("second", "b"), ("third", "c")] {
            conversation.push(Message::user(prompt));
            conversation.push(Message::assistant(answer));
        }
        conversation
    }

    /// A summary is a paraphrase, and the exchange a session is in the middle of is the one that
    /// can least afford to be paraphrased.
    #[test]
    fn compaction_keeps_the_most_recent_exchanges_word_for_word() {
        let mut conversation = three_exchanges();
        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they asked about the first thing");

        let kept: Vec<String> = conversation
            .messages()
            .iter()
            .map(|message| message.content.text())
            .collect();
        assert_eq!(kept.len(), 5, "{kept:?}");
        assert!(kept[0].starts_with(COMPACTED_PREFIX), "{kept:?}");
        assert_eq!(&kept[1..], ["second", "b", "third", "c"]);
    }

    /// The invariant the cut point exists for. `with_system` answers a call nothing answered by
    /// looking only at the run of results immediately after it, so an assistant message parted
    /// from its results leaves a request the server refuses and a record that says a tool both
    /// ran and did not.
    #[test]
    fn compaction_never_separates_a_call_from_its_results() {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("first"));
        conversation.push(Message::assistant_calling(
            "looking",
            vec![a_call("read_file", r#"{"path":"notes.md"}"#)],
        ));
        conversation.push(Message::tool_result("call-1", "the notes"));
        conversation.push(Message::assistant("done"));
        conversation.push(Message::user("second"));
        conversation.push(Message::assistant("b"));
        conversation.push(Message::user("third"));
        conversation.push(Message::assistant("c"));

        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they asked about the notes");

        let sent = conversation.with_system("be careful");
        assert!(
            !sent
                .iter()
                .any(|m| m.content.text().contains("did not run")),
            "a round was cut in half: {sent:?}"
        );
        assert!(
            !sent.iter().any(|m| m.tool_call_id.is_some()),
            "an answer outlived the call it answered: {sent:?}"
        );
    }

    /// Compacting a conversation that is all recent would trade the exact words of what is being
    /// worked on for a paraphrase of it, and save nothing worth having.
    #[test]
    fn a_conversation_with_nothing_but_recent_exchanges_is_not_compacted() {
        let mut conversation = Conversation::new();
        conversation.push(Message::user("first"));
        conversation.push(Message::assistant("a"));
        conversation.push(Message::user("second"));
        assert_eq!(conversation.compaction_boundary(), None);
    }

    /// There is nothing left to give up, and saying so beats spending a request to summarise a
    /// summary into a summary.
    #[test]
    fn a_head_that_is_only_an_earlier_summary_is_not_compacted_again() {
        let mut conversation = three_exchanges();
        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they asked about the first thing");

        assert_eq!(conversation.compaction_boundary(), None);
    }

    /// And it starts again once there is something new behind the summary, or a long session
    /// would compact once and then grow forever.
    #[test]
    fn a_conversation_that_carries_on_after_a_summary_compacts_again() {
        let mut conversation = three_exchanges();
        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they asked about the first thing");

        conversation.push(Message::user("fourth"));
        conversation.push(Message::assistant("d"));
        assert!(conversation.compaction_boundary().is_some());
    }

    /// What compaction shortens is the request, not the record. A person resuming a session they
    /// spent an afternoon on must not find their own earlier prompts missing from it.
    #[test]
    fn what_compaction_took_out_of_the_request_is_still_recounted_to_the_person() {
        let mut conversation = three_exchanges();
        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they asked about the first thing");

        assert_eq!(
            conversation.recounted(),
            vec![
                Said::User("first".to_string()),
                Said::Assistant("a".to_string()),
                Said::User("second".to_string()),
                Said::Assistant("b".to_string()),
                Said::User("third".to_string()),
                Said::Assistant("c".to_string()),
            ]
        );
    }

    /// The summary is written for the planner. Drawn in a transcript it would read as a prompt
    /// the user never typed, on top of the exchange it is standing in for.
    #[test]
    fn the_summary_is_not_shown_as_something_the_user_said() {
        let mut conversation = three_exchanges();
        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they asked about the first thing");

        assert!(
            !conversation.recounted().contains(&Said::User(format!(
                "{COMPACTED_PREFIX}\n\nthey asked about the first thing"
            ))),
            "the summary was drawn as a prompt"
        );
    }

    /// Slots are written once, so a name handed out twice is a collision rather than a muddle.
    #[test]
    fn compaction_does_not_rewind_the_reference_counter() {
        let mut conversation = three_exchanges();
        let _ = conversation.next_reference();
        let _ = conversation.next_reference();

        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they read two files");

        assert_eq!(conversation.next_reference(), SlotId::new("ref:2"));
    }

    /// The quarantine holds the only copy of what a surviving reference names. Dropping it to
    /// save room would leave the planner holding names for content that no longer exists, which
    /// is the failure a resume already has and compaction has no excuse for.
    #[test]
    fn a_reference_minted_before_compaction_still_names_its_content_after_it() {
        let mut conversation = three_exchanges();
        let slot = conversation.next_reference();
        conversation
            .quarantine()
            .writer_for(slot.clone(), Label::untrusted_private())
            .expect("a writer")
            .write("what nobody may read")
            .expect("written");

        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they read a file");

        assert!(
            conversation.quarantine().take_for_effect(&slot).is_ok(),
            "compaction dropped the content behind a live reference"
        );
    }

    /// The names are in the summary and in the retained messages, and nothing in either says
    /// whether they still work, so without this the planner finds out by being refused.
    #[test]
    fn compaction_says_which_references_still_work() {
        let mut conversation = three_exchanges();
        let slot = conversation.next_reference();
        conversation
            .quarantine()
            .writer_for(slot, Label::untrusted_private())
            .expect("a writer")
            .write("what nobody may read")
            .expect("written");

        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they read a file");

        let note = conversation.messages()[0].content.text();
        assert!(note.contains("ref:0"), "{note}");
    }

    /// A name minted for content the planner turned out to be allowed to read never became a
    /// slot. Claiming it still names something would send the planner after nothing.
    #[test]
    fn a_reference_that_was_never_quarantined_is_not_claimed_to_be_live() {
        let mut conversation = three_exchanges();
        let _ = conversation.next_reference();

        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they read a file they were allowed to see");

        let note = conversation.messages()[0].content.text();
        assert!(!note.contains("ref:0"), "{note}");
    }

    /// A session that outlives the process must come back compacted, and with the archive that
    /// makes the shortening free for the person reading it.
    #[test]
    fn a_compacted_conversation_survives_being_written_down() {
        let mut conversation = three_exchanges();
        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they asked about the first thing");

        let restored = Conversation::restored(conversation.snapshot());
        assert_eq!(restored.len(), 5);
        assert!(
            restored.messages()[0]
                .content
                .as_text()
                .is_some_and(|text| text.starts_with(COMPACTED_PREFIX))
        );
        assert_eq!(restored.recounted().len(), 6);
    }

    /// Session files predate this field. Someone resuming yesterday's work should not be told
    /// their session is unreadable because a newer build wanted a key it had never written.
    #[test]
    fn a_session_file_written_before_compaction_existed_still_reads() {
        let stored = r#"{"messages":[{"role":"user","content":"what is 2 + 2?"}],
                         "context":"trusted","references":0}"#;
        let snapshot: Snapshot = serde_json::from_str(stored).expect("an older session file");
        let restored = Conversation::restored(snapshot);

        assert_eq!(restored.len(), 1);
        assert_eq!(restored.recounted().len(), 1);
    }

    /// Compaction is not a fresh session. Nothing in it has un-read what the conversation read,
    /// and a summary that came back trusted would be the one upgrade this repository forbids.
    #[test]
    fn compaction_does_not_restore_integrity_the_conversation_had_lost() {
        let mut conversation = three_exchanges();
        conversation.observed(Integrity::Untrusted);

        let boundary = conversation
            .compaction_boundary()
            .expect("something to compact");
        conversation.compacted(boundary, "they asked about the first thing");

        assert_eq!(conversation.context(), Integrity::Untrusted);
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
                archive: Vec::new(),
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
