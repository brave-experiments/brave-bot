//! Telling the interface what a turn is doing, while it does it.
//!
//! Distinct from [`crate::confirm`], and the difference is consent. A write asks, blocks, and
//! must refuse if nobody can answer. Progress announces: there is no question, no reply, and no
//! answer that could change what happens. So this returns nothing, and a listener that has gone
//! away is not an error.
//!
//! That asymmetry decides the failure behaviour. A closed channel means a write must not happen,
//! but a task list nobody is drawing is merely unseen, and failing the turn over it would let the
//! display outrank the work.

use crate::diff::Change;
use bravebot_core::todo::Row;
use bravebot_i18n::t;

/// One thing the turn did, shaped for the person watching.
///
/// Every string in here has already been through the display gate, exactly as
/// [`crate::confirm::WriteRequest`] has, so nothing downstream reasons about labels. The
/// release is what the gate exists for: a screen is one of the three destinations untrusted
/// content is allowed to reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    /// What is being done, in the driver's own word.
    ///
    /// A literal chosen by the dispatch table rather than anything the model wrote, so a call
    /// cannot describe itself as something gentler than it is.
    pub verb: &'static str,
    /// What is being acted on, as the model named it. Empty where there is nothing to name.
    pub target: String,
    /// What came of it, in a few words. `None` while the call is still running, which is what
    /// makes an unfinished line distinguishable from one that finished with nothing to say.
    pub note: Option<String>,
    /// Whether the call was refused or failed, so the line can be coloured as such.
    ///
    /// Set by the driver from which branch it took, never read back out of the note: the note
    /// is prose, and matching on prose is how a message that merely mentions a refusal becomes
    /// one.
    pub failed: bool,
    /// The change a write made, for showing beneath the line. Empty for everything else.
    pub changes: Vec<Change>,
    /// Whether those lines are content nobody vouched for.
    ///
    /// Drawn with the same mark the transcript puts on everything the model was not allowed to
    /// read, so one convention covers every place untrusted bytes reach a screen.
    pub untrusted: bool,
}

impl Activity {
    /// A call that has begun and has not finished.
    pub fn running(verb: &'static str, target: impl Into<String>) -> Self {
        Self {
            verb,
            target: target.into(),
            note: None,
            failed: false,
            changes: Vec::new(),
            untrusted: false,
        }
    }

    /// The same call, finished, with what came of it.
    pub fn done(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// The same call, refused or failed, with why.
    pub fn failed(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self.failed = true;
        self
    }

    /// Attach the change a write made.
    pub fn with_changes(mut self, changes: Vec<Change>) -> Self {
        self.changes = changes;
        self
    }

    /// Say that the lines beneath this call are content nobody vouched for.
    pub fn marked_untrusted(mut self, untrusted: bool) -> Self {
        self.untrusted = untrusted;
        self
    }

    /// Whether this line is still waiting on the call it describes.
    pub fn is_running(&self) -> bool {
        self.note.is_none()
    }

    /// The line as one string, for a display with nowhere to put the parts separately.
    pub fn line(&self) -> String {
        if self.target.is_empty() {
            self.verb.to_string()
        } else {
            format!("{}({})", self.verb, self.target)
        }
    }
}

/// What the turn is waiting on.
///
/// The driver's own words, chosen from the round number and nothing else. A wait that says what
/// it is a wait for is the difference between a slow turn and an apparently stuck one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The first call to the model. It has the task and nothing else, so what it is doing is
    /// working out what to do.
    Planning,
    /// A later call, with tool results in hand.
    Thinking,
    /// The conversation is being summarised so that it stops growing.
    ///
    /// Worth its own word for the same reason as reconnecting: nothing is being worked out, and
    /// a pause the user cannot account for is the difference between a slow turn and a stuck one.
    Compacting,
    /// The request failed in transit and is being sent again.
    ///
    /// Worth its own word because the pause looks like the others and is not one: nothing is
    /// being worked out, and what the model had written has been thrown away.
    Reconnecting,
}

impl Phase {
    /// Which phase a round is, counting rounds already taken.
    pub fn of_round(rounds_taken: usize) -> Self {
        if rounds_taken == 0 {
            Self::Planning
        } else {
            Self::Thinking
        }
    }

    /// The word to show, as a verb someone can read beside a spinner.
    pub fn word(&self) -> &'static str {
        match self {
            Self::Planning => "Planning",
            Self::Thinking => "Thinking",
            Self::Compacting => "Compacting",
            Self::Reconnecting => "Reconnecting",
        }
    }
}

/// How long ago something happened, in the words a person says it in.
///
/// Lives here rather than in the interface because two things need it and a phrase written twice
/// is a phrase that will disagree with itself: the list of sessions saying when one was last
/// touched, and a write saying how old the file it replaced was.
/// Deliberately not from a catalog. This reads back as part of the note a write answers the
/// planner with, in tools.rs, so the words are interface to the model rather than prose for a
/// person. The session list has its own, in the interface, which is translated.
pub fn how_long_ago(age: std::time::Duration) -> String {
    let seconds = age.as_secs();

    let (count, unit) = match seconds {
        0..=59 => return "just now".to_string(),
        60..=3_599 => (seconds / 60, "minute"),
        3_600..=86_399 => (seconds / 3_600, "hour"),
        86_400..=2_591_999 => (seconds / 86_400, "day"),
        _ => (seconds / 2_592_000, "month"),
    };

    if count == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{count} {unit}s ago")
    }
}

/// Something that can be told about progress.
///
/// A trait so a turn does not depend on a terminal: the interactive session draws, a one-shot run
/// ignores, and tests record.
/// How far the content in a [`Shown`] block reaches.
///
/// There is more than one model in a turn, so "not shown to the model" says nothing useful. The
/// driver is not a model at all: it carries bytes it may not read, and has no context to put
/// them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Never in the planner's context. An isolated processor can be sent to read it, if the
    /// planner names it.
    NotThePlanner,
    /// In no model's context at all: nothing can be sent to read it, and it is not part of any
    /// file. What a processor says about its own work is this.
    NoModel,
}

impl Reach {
    pub fn describe(self) -> &'static str {
        match self {
            Self::NotThePlanner => t!(reach_not_the_planner),
            Self::NoModel => t!(reach_no_model),
        }
    }
}

/// Quarantined content, released so the person watching can see it.
///
/// The planner is never shown this and neither is a processor: it goes to a screen and stops
/// there. That is not a hole in the confinement, it is what the confinement is for. The user owns
/// the directory and is entitled to know what their agent is working on; what must not happen is
/// those bytes reaching a model's context, and a screen is not a context.
///
/// Marked wherever it is drawn, and marked structurally rather than by a line of text the content
/// could imitate. See the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Where it came from, in words a person can act on: a path, or what produced it.
    pub origin: String,
    /// Which contexts this is kept out of.
    pub reach: Reach,
    /// The label it carries, in the same short form the trail uses.
    pub label: String,
    /// The first lines of it, each already trimmed to a sensible width.
    pub preview: Vec<String>,
    /// How many lines there are altogether, so a preview can say what it left out.
    pub lines: usize,
}

/// Where a tool's result went, which is the thing a person cannot otherwise tell.
///
/// "Read(index.html)" says nothing about whether the model can now read that file, and the
/// difference is the whole design: one of these puts a file in front of the model, one puts it
/// somewhere only an isolated processor can be sent, and one reads nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// Into the planner's context. The model has read this.
    Context,
    /// Into a slot. Nothing has read it but the driver, and the only thing that can be sent to
    /// read it is an isolated processor.
    Quarantined,
    /// Nowhere. The file is named and has not been opened.
    Reserved,
}

impl Landing {
    /// How it reads at the end of a line about a call.
    ///
    /// Names which context, because there is more than one kind of model here and the driver is
    /// not one of them: the planner is the model holding the conversation, and a processor is an
    /// isolated model that is handed slots and nothing else. "The model" answers neither
    /// question a person is asking.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Context => t!(landed_in_the_planner),
            Self::Quarantined => t!(landed_quarantined),
            Self::Reserved => t!(landed_reserved),
        }
    }
}

pub trait Reporter {
    /// The task list changed. Rows are already shaped for display and released.
    fn todos(&mut self, rows: Vec<Row>);

    /// The model has written more of its reply.
    ///
    /// A count and nothing else. The reply is untrusted model output, so passing the text here
    /// would put untrusted content in the driver's hands; how much was written is not content.
    fn output_tokens(&mut self, _written: u64) {}

    /// The turn is waiting on the model again.
    ///
    /// Sent before each request, so the wait before the first tool call says what it is: the
    /// model working out a plan, which is the longest silence in a turn and used to be the
    /// least explained.
    fn phase(&mut self, _phase: Phase) {}

    /// The model said something on its way to calling more tools.
    ///
    /// Released model output, so it is shown rather than acted on, exactly like the final
    /// reply. This text used to be discarded: a turn that explained each step as it went had
    /// every one of those explanations thrown away, and the user saw a spinner instead.
    ///
    /// Sent whether or not there is anything in it. Deciding that from the text would be the
    /// driver taking a decision from untrusted bytes, and an empty line is the interface's to
    /// leave undrawn.
    fn narration(&mut self, _text: String) {}

    /// The reply as the model writes it, in the words added since the last call.
    ///
    /// Released for a screen and nowhere else, exactly as the finished text is. Sent whether or
    /// not it is empty: whether there is anything to draw is a question about the text, and the
    /// driver does not get to ask questions about it. An interface that draws this replaces what
    /// it drew when the round's finished text arrives, so nothing is said twice.
    fn streaming(&mut self, _text: String) {}

    /// Say what the turn found before it started: which standing instructions and skills loaded,
    /// and which did not.
    ///
    /// Sent when it is learned, which is before the first request goes out. The outcome carries
    /// the same lines for a caller with no live display, but a caller that has one and waits for
    /// the outcome draws them after every tool line, describing what the turn began with as
    /// though it were the last thing that happened.
    ///
    /// The driver's own words about a file it enumerated, never anything read out of one, so
    /// there is nothing here for a display gate to release.
    fn notice(&mut self, _text: String) {}

    /// Untrusted content, for the person watching to read.
    ///
    /// Sent whenever a result is quarantined, which is exactly when the planner is told a
    /// reference and nothing else. The user is not the planner: they own the workspace, they are
    /// the one who can tell whether the agent is working on the right file, and leaving them with
    /// "2 files, quarantined" told them nothing they could use.
    fn quarantined(&mut self, _shown: Shown) {}

    /// Where the result of the call just finished ended up.
    ///
    /// Sent after the kernel has decided, which is why it is not part of the finished call: what
    /// happens to a result is settled by its label, after the tool that produced it has returned.
    fn landed(&mut self, _landing: Landing) {}

    /// A tool call has begun.
    ///
    /// Sent before the call runs, so a slow one is visible while it is slow rather than only
    /// once it is over. That is the whole point: a turn that reads twenty files used to show
    /// nothing at all until it finished.
    fn tool_started(&mut self, _activity: Activity) {}

    /// The call [`Reporter::tool_started`] last announced has finished.
    ///
    /// Paired by position rather than by an identifier because dispatch runs one call at a
    /// time: there is never a second call in flight for this to be ambiguous between.
    fn tool_finished(&mut self, _activity: Activity) {}
}

/// Discards every report.
///
/// The right behaviour where there is no live display: a one-shot command, a pipeline. Unlike
/// refusing a write, discarding a progress report costs nothing, since it was never going to
/// change what the turn did.
#[derive(Debug, Default)]
pub struct IgnoreReports;

impl Reporter for IgnoreReports {
    fn todos(&mut self, _rows: Vec<Row>) {}
}

/// Keeps what it was told, for tests.
#[derive(Debug, Default)]
pub struct RecordingReporter {
    /// Every update in order, so a test can assert on the sequence rather than the end state.
    pub updates: Vec<Vec<Row>>,
    /// Every output-token count reported, in order.
    pub written: Vec<u64>,
    /// Every tool call announced as starting, in order.
    pub started: Vec<Activity>,
    /// Every tool call announced as finished, in order.
    pub finished: Vec<Activity>,
    /// Every phase the turn entered, in order.
    pub phases: Vec<Phase>,
    /// Everything the model said between tool calls, in order.
    pub narration: Vec<String>,
    /// Every fragment of a reply as it arrived, in order.
    pub streamed: Vec<String>,
    /// What loaded and what did not, in order.
    pub notices: Vec<String>,
    /// Quarantined content released for the screen.
    pub shown: Vec<Shown>,
    /// Where each result went.
    pub landed: Vec<Landing>,
}

impl Reporter for RecordingReporter {
    fn todos(&mut self, rows: Vec<Row>) {
        self.updates.push(rows);
    }

    fn output_tokens(&mut self, written: u64) {
        self.written.push(written);
    }

    fn phase(&mut self, phase: Phase) {
        self.phases.push(phase);
    }

    fn narration(&mut self, text: String) {
        self.narration.push(text);
    }

    fn streaming(&mut self, text: String) {
        self.streamed.push(text);
    }

    fn notice(&mut self, text: String) {
        self.notices.push(text);
    }

    fn tool_started(&mut self, activity: Activity) {
        self.started.push(activity);
    }

    fn tool_finished(&mut self, activity: Activity) {
        self.finished.push(activity);
    }

    fn quarantined(&mut self, shown: Shown) {
        self.shown.push(shown);
    }

    fn landed(&mut self, landing: Landing) {
        self.landed.push(landing);
    }
}

/// The driver's word for what a tool does.
///
/// Chosen from the tool's name, which dispatch already matches on, so this decides nothing new.
/// A literal rather than the raw name because the line is read by a person: "Read(src/main.rs)"
/// says what happened and "read_file" says what was typed.
pub(crate) fn verb_for(tool: &str) -> &'static str {
    match tool {
        "read_file" => t!(verb_read_file),
        "list_files" => t!(verb_list_files),
        "search" => t!(verb_search),
        "write_file" => t!(verb_write_file),
        "edit_file" => t!(verb_edit_file),
        "todo_write" => t!(verb_todo_write),
        // Named for what it is rather than for what it does: every one of these is a model
        // with no tools, no memory and one round, and a person watching a line go by should not
        // have to remember which of the verbs meant that.
        "spawn_processor" => t!(verb_spawn_processor),
        "vet_content" => t!(verb_vet_content),
        "load_skill" => t!(verb_load_skill),
        "ask_user" => t!(verb_ask_user),
        "run" => t!(verb_run),
        "read_output" => t!(verb_read_output),
        _ => t!(verb_unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_core::todo::{Item, List, Status, rows};

    #[test]
    fn a_recording_reporter_keeps_each_update_in_order() {
        let mut reporter = RecordingReporter::default();
        reporter.todos(rows(&List::new(vec![Item::new("one", Status::Pending)])));
        reporter.todos(rows(&List::new(vec![Item::new("one", Status::Done)])));

        assert_eq!(reporter.updates.len(), 2);
        assert!(!reporter.updates[0][0].struck());
        assert!(reporter.updates[1][0].struck());
    }

    /// Nothing to draw is not a failure. A reporter has no way to refuse, by design: there is no
    /// return value it could refuse with.
    #[test]
    fn ignoring_reports_is_infallible() {
        IgnoreReports.todos(rows(&List::new(vec![Item::new("x", Status::Active)])));
        IgnoreReports.tool_started(Activity::running("Read", "src/main.rs"));
        IgnoreReports.tool_finished(Activity::running("Read", "src/main.rs").done("12 lines"));
    }

    /// The distinction the display draws everything else from: a line with no note is a call
    /// still in flight, and one with a note is over.
    #[test]
    fn an_activity_is_running_until_it_has_a_note() {
        let started = Activity::running("Read", "src/main.rs");
        assert!(started.is_running());
        assert!(!started.clone().done("12 lines").is_running());
        assert!(!started.failed("refused").is_running());
    }

    /// A refusal has to be distinguishable from a success without reading the note, or the
    /// interface would be matching on prose to decide what colour to draw.
    #[test]
    fn a_refusal_is_marked_as_one_rather_than_described_as_one() {
        let refused = Activity::running("Update", "a.rs").failed("refused: not approved");
        assert!(refused.failed);
        assert!(!Activity::running("Update", "a.rs").done("+1 -0").failed);
    }

    /// The first wait is the one that needs explaining: the model has the task and nothing
    /// else, and there is no tool call yet to show for it.
    #[test]
    fn the_first_round_is_planning_and_the_rest_are_not() {
        assert_eq!(Phase::of_round(0), Phase::Planning);
        assert_eq!(Phase::of_round(1), Phase::Thinking);
        assert_eq!(Phase::of_round(9), Phase::Thinking);
        assert_ne!(Phase::Planning.word(), Phase::Thinking.word());
    }

    #[test]
    fn ages_read_the_way_a_person_says_them() {
        use std::time::Duration;
        assert_eq!(how_long_ago(Duration::from_secs(3)), "just now");
        assert_eq!(how_long_ago(Duration::from_secs(60)), "1 minute ago");
        assert_eq!(how_long_ago(Duration::from_secs(13 * 60)), "13 minutes ago");
        assert_eq!(how_long_ago(Duration::from_secs(2 * 3_600)), "2 hours ago");
        assert_eq!(how_long_ago(Duration::from_secs(86_400)), "1 day ago");
        assert_eq!(
            how_long_ago(Duration::from_secs(40 * 86_400)),
            "1 month ago"
        );
    }

    #[test]
    fn a_line_names_what_was_acted_on() {
        assert_eq!(
            Activity::running("Read", "src/main.rs").line(),
            "Read(src/main.rs)"
        );
    }

    /// Some work has nothing to name, and an empty pair of brackets reads as a bug rather than
    /// as an absent target.
    #[test]
    fn a_line_with_nothing_to_name_is_the_verb_alone() {
        assert_eq!(Activity::running("Plan", "").line(), "Plan");
    }

    /// Every tool the model is offered needs a word of its own. Without this a new tool
    /// shows up in the transcript as the fallback, which tells the user nothing.
    #[test]
    fn every_offered_tool_has_its_own_verb() {
        for tool in crate::tools::available() {
            let name = &tool.function.name;
            assert_ne!(
                verb_for(name),
                verb_for("something nobody wrote"),
                "{name} has no verb of its own"
            );
        }
    }
}
