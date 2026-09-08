//! Session state, kept separate from rendering so it can be tested without a terminal.
//!
//! A session is a sequence of independent turns. It holds transcript and input state for
//! display; it does **not** hold a policy. Each turn constructs its own, which is what
//! stops routing from one turn leaking into the next as untrusted content accumulates.

use crate::audit::TrailLine;
use bravebot_agent::report::{Activity, Landing, Phase, Printed, Reported, Shown};
use bravebot_aichat::protocol::Effort;
use bravebot_core::event::Event;
use bravebot_i18n::t;
use std::time::{Duration, Instant};

/// How many newlines a paste carries before it is folded behind a marker.
///
/// Counted in newlines rather than in lines so that three lines with nothing after the last of
/// them is the first paste to fold: the third newline is the point where the box was going to grow
/// past what a prompt is meant to look like.
const FOLD_AT_NEWLINES: usize = 3;

/// Text with the line endings every clipboard uses turned into the one the box draws.
fn normalised(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// How many lines a paste is, as a person reading it would count them.
///
/// A trailing newline ends the last line rather than starting another, so text copied with the end
/// of its last line does not claim an empty line that nobody can see.
fn lines_in(text: &str) -> usize {
    let newlines = text.matches('\n').count();
    if text.ends_with('\n') {
        newlines
    } else {
        newlines + 1
    }
}

/// Who produced a transcript entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    /// The user's prompt. Trusted input.
    User,
    /// The assistant's reply. Untrusted model output, shown but never acted on.
    Assistant,
    /// A note from the program itself: an error, a refusal, a status.
    System,
    /// A tool call the turn made. Shown as it happens, and kept afterwards.
    Tool,
    /// A command the user typed in shell mode. Trusted input, like their prompts.
    Shell,
    /// What such a command printed.
    ///
    /// Drawn plainly rather than styled: it is a terminal's output and the user is reading it as
    /// one, so markdown would be a misreading and a marker on every line would be noise.
    Output,
    /// A delegate the turn started, with its own work drawn underneath it.
    Delegate,
}

/// One delegate's work, drawn where the call that started it would have been.
///
/// A delegate is a second planner with a run of its own, and several are going at once, so its
/// lines are kept together rather than interleaved with the turn's. The block where the call
/// happened draws the last few of them; the rest are here for the mode that opens over it.
///
/// Nothing here reaches a model. The turn is told the report and nothing else, which is the point
/// of delegating; these are the same lines going to a screen instead.
#[derive(Debug, Clone)]
pub struct Delegate {
    /// Which one it is, as the driver numbered it.
    pub id: bravebot_agent::report::DelegateId,
    /// Which kind it is, in the driver's own word.
    pub kind: &'static str,
    /// What it was asked to do, as the planner wrote it.
    pub task: String,
    /// What it has done, oldest first, back as far as is kept.
    pub lines: Vec<Entry>,
    /// How many it has done in all, which is more than are kept once it has run long enough.
    pub calls: usize,
    /// What the turn was told when it finished. `None` while it is still working, which is what
    /// tells a delegate that is running from one that answered.
    pub note: Option<String>,
    /// What it handed back, where it handed back anything. A delegate that could not finish
    /// reported nothing.
    ///
    /// Held apart from the note because the two answer different questions: the note is the
    /// driver's sentence about how the run ended, and this is the delegate's own about what it
    /// found. Which shape it takes was settled by the gate that decided what the planner got.
    pub reported: Option<Reported>,
    /// Whether it ended by failing, so the line saying so can be coloured as such.
    pub failed: bool,
}

/// A command this session ran, and what it printed.
///
/// Kept whether or not the planner was allowed to read it. The person owns the directory and is
/// entitled to read what their agent ran; what must not happen is those bytes reaching a model's
/// context, and a screen is not a context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The command, as the plan the person endorsed showed it.
    pub command: String,
    /// What it printed, as far back as is kept.
    pub lines: Vec<String>,
    /// How many lines there were altogether, so the view can say what it left out.
    pub total: usize,
    /// Whether the planner was allowed to read it.
    pub read_by_the_planner: bool,
    /// How the command ended, as the driver said it from the exit codes and the clock.
    pub outcome: bravebot_agent::report::Outcome,
}

/// Something the delegate view can open.
///
/// One list rather than two, because a person pressing the key is asking to see work that is not
/// in the transcript, and which kind of work it was is a property of the row rather than a reason
/// for a second key.
#[derive(Debug, Clone, Copy)]
pub enum Watched<'a> {
    /// A delegate's own work.
    Delegate(&'a Delegate),
    /// What a command printed.
    Output(&'a Output),
}

/// How many of a delegate's own lines the block where it started draws.
///
/// Enough to see that something is happening and roughly what, which is all the block is for.
/// Somebody wanting more than that opens the delegate itself.
pub const DELEGATE_SHOWN: usize = 3;

/// How many of a delegate's own lines are kept for the screen.
///
/// A bound rather than the whole of it: a delegate that runs long enough produces an unbounded
/// number of lines, and these are held in memory for a person who may never look. Far enough back
/// to cover the work somebody opens a delegate to ask about.
const DELEGATE_KEPT: usize = 400;

impl Delegate {
    /// Whether this one is still working.
    pub fn is_running(&self) -> bool {
        self.note.is_none()
    }

    /// The last few of its lines, which is what the block where it started draws.
    pub fn latest(&self) -> &[Entry] {
        &self.lines[self.lines.len().saturating_sub(DELEGATE_SHOWN)..]
    }

    /// Keep one more of its lines, dropping the oldest where there are already enough.
    fn keep(&mut self, entry: Entry) {
        self.calls += 1;
        self.lines.push(entry);
        if self.lines.len() > DELEGATE_KEPT {
            self.lines.remove(0);
        }
    }
}

/// One entry in the transcript.
#[derive(Debug, Clone)]
pub struct Entry {
    pub speaker: Speaker,
    pub text: String,
    /// The audit trail recorded while producing this entry, shown when the trail is visible.
    ///
    /// Already in the words it is drawn in, because an entry replayed from a stored session has
    /// no events behind it: what the audit file holds is a record of what a gate decided, not the
    /// decision. See [`crate::audit::TrailLine`].
    pub trail: Vec<TrailLine>,
    /// The task list as it stood when this entry was made, if the turn kept one.
    ///
    /// Held on the entry rather than in one place so the scrollback shows what each turn did.
    /// A live list belongs to the turn in flight and goes here when that turn ends.
    pub todos: Vec<bravebot_core::todo::Row>,
    /// The call this entry describes, for a [`Speaker::Tool`] entry.
    ///
    /// Carries the note and the hunks separately from `text` so the interface can style them
    /// without parsing anything back out of a formatted line.
    pub activity: Option<Activity>,
    /// Where this call's result went: into the model's context, into a slot, or nowhere.
    ///
    /// The line says what was read; this says who can read it, which is the part a person
    /// cannot work out from the outside and the part the whole design turns on.
    pub landing: Option<Landing>,
    /// Quarantined content this call produced, for the person watching.
    ///
    /// Kept apart from `text` because it is drawn apart: it is the one thing on the screen the
    /// model was not allowed to read, and it is marked as such by the renderer rather than by
    /// anything in the bytes, which could say whatever they liked.
    pub shown: Option<Shown>,
    /// The delegate this entry stands for, for a [`Speaker::Delegate`] entry.
    ///
    /// Its own lines live here rather than in the transcript around it. Several delegates work at
    /// once, so lines interleaved with the turn's could not be read in either direction: whose
    /// each one was would be a guess from the words, and the words are prose a model wrote.
    pub delegate: Option<Delegate>,
}

impl Entry {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::User,
            text: text.into(),
            trail: Vec::new(),
            todos: Vec::new(),
            landing: None,
            shown: None,
            activity: None,
            delegate: None,
        }
    }

    pub fn assistant(text: impl Into<String>, trail: Vec<TrailLine>) -> Self {
        Self {
            speaker: Speaker::Assistant,
            text: text.into(),
            trail,
            todos: Vec::new(),
            landing: None,
            shown: None,
            activity: None,
            delegate: None,
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::System,
            text: text.into(),
            trail: Vec::new(),
            todos: Vec::new(),
            landing: None,
            shown: None,
            activity: None,
            delegate: None,
        }
    }

    /// A command the user ran in shell mode, echoed as they typed it.
    pub fn shell(line: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::Shell,
            text: line.into(),
            trail: Vec::new(),
            todos: Vec::new(),
            landing: None,
            shown: None,
            activity: None,
            delegate: None,
        }
    }

    /// What such a command printed.
    pub fn output(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::Output,
            text: text.into(),
            trail: Vec::new(),
            todos: Vec::new(),
            landing: None,
            shown: None,
            activity: None,
            delegate: None,
        }
    }

    /// One tool call, as it stands.
    ///
    /// Made while the call is still running and replaced when it finishes, which is what puts
    /// a slow call on the screen while it is slow rather than only once it is over.
    pub fn tool(activity: Activity) -> Self {
        Self {
            speaker: Speaker::Tool,
            text: activity.line(),
            trail: Vec::new(),
            todos: Vec::new(),
            landing: None,
            shown: None,
            activity: Some(activity),
            delegate: None,
        }
    }

    /// One call read back out of a stored session.
    ///
    /// No [`Activity`], because a stored session records that the call happened and not what came
    /// of it. Giving it one would mean choosing an outcome, and every choice available is a
    /// claim the record does not support: `running` says it never finished, and `done` says it
    /// succeeded. The line alone is what is known, and the interface draws it as such.
    pub fn recalled_tool(line: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::Tool,
            text: line.into(),
            trail: Vec::new(),
            todos: Vec::new(),
            landing: None,
            shown: None,
            activity: None,
            delegate: None,
        }
    }

    /// Attach the task list the turn finished with.
    pub fn with_todos(mut self, todos: Vec<bravebot_core::todo::Row>) -> Self {
        self.todos = todos;
        self
    }
}

/// A prompt typed while a turn was running, waiting for it to end.
///
/// Settled when it was queued rather than when it is sent, because what it names is what the box
/// held at that moment. A file the user took off the line afterwards was never part of this
/// prompt, and one they added belongs to whatever they type next.
#[derive(Debug, Clone)]
pub struct Queued {
    /// The line, as it was typed.
    pub prompt: String,
    /// Files it named, settled at the moment it was queued.
    attached: Vec<Attached>,
    /// Pictures it named, settled at the same moment and for the same reason.
    pasted: Vec<AttachedImage>,
}

/// What the session is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting for input.
    Idle,
    /// A turn is in flight. Input is refused so a second turn cannot start mid-flight and
    /// share the first one's state.
    Working,
    /// A command the user typed in shell mode is running.
    ///
    /// Distinct from [`Status::Working`] because the two are not the same wait: a turn spends
    /// tokens and reports phases, and a command does neither, so the indicator that suits one is
    /// mostly empty fields for the other.
    Running,
    /// The user asked to leave.
    Quitting,
}

/// What the last turn came to, for the line that says it is over.
///
/// Kept rather than recomputed from the transcript because the figures are about the turn, and the
/// transcript is about what was said. A turn that spent forty rounds and one that spent one look
/// the same in scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finished {
    /// Which turn it was, counted the way the session counts them.
    pub turn: usize,
    /// What the whole turn cost, every round added together.
    pub tokens: u64,
    /// How long it took, wall clock.
    pub took: Duration,
    /// Whether it ended by failing, which is a different thing to report.
    pub failed: bool,
}

/// What a half-typed line could still become.
///
/// One kind at a time: a command is the whole line and a file reference is its last word, so the
/// list is never a mixture and the keys that walk it never have to ask which they are walking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offered {
    /// Nothing is being typed towards, so the list is closed.
    Nothing,
    Commands(Vec<crate::app::Command>),
    Files(Vec<crate::entries::Entry>),
    /// Every key and marker, listed under the box. Not a completion: there is nothing to choose,
    /// which is why the keys that walk a list leave this one alone.
    Shortcuts,
}

/// What kind of character one is, for working out where a word begins and ends.
///
/// Three kinds rather than two, because vi's `w` treats punctuation as a word of its own: in
/// `src/main.rs` the slashes are not part of either name, which is what makes `diw` on one of them
/// take the slash alone. `W` uses the same machinery with punctuation folded into `Word`, and that is
/// the whole of the difference between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Blank,
    Word,
    Punctuation,
}

/// What a yank or a delete took, and whether it was whole lines.
///
/// The distinction is what `p` needs: a yanked word goes back beside the caret, and a yanked line goes
/// back as a line of its own. Without it, `yy` then `p` would splice a sentence into the middle of
/// another one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Yanked {
    text: String,
    lines: bool,
}

/// A binding vi spells with a letter, which the key handler answers as though the key had arrived.
///
/// These reach past the line: at the ends of the input the row keys walk the prompt history and then
/// scroll the transcript, and the search is a mode standing over the whole box. So the letter is
/// translated and the existing arms answer it, rather than a second copy of that ladder being written
/// for three letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spelled {
    /// `k`, which is Up.
    Up,
    /// `j`, which is Down.
    Down,
    /// `/`, which is the chord that searches the prompts already sent.
    SearchPrompts,
}

/// A file dropped on the box, and the marker standing for it in the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    /// What the user sees in the line, such as `[Image #1]`.
    pub marker: String,
    /// The name to give the task, already checked against what the workspace can open.
    pub name: String,
    /// The path as the user's filesystem names it, for showing them what they attached.
    pub shown: String,
    pub kind: crate::dropped::Kind,
}

/// A picture pasted into the line being typed.
///
/// Separate from [`Attached`] because the two arrive by different routes and only one of them has
/// a path: a dropped file is read out of the workspace, where the trust map has something to say
/// about it, and a paste is bytes that never touched the filesystem. They share the marker
/// numbering, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedImage {
    /// The text standing for it in the prompt, as `[Image #1]`.
    ///
    /// Held rather than derived from a position, because the line is edited around it: the picture
    /// belongs to the words the marker sits in, and finding it again by counting would go wrong the
    /// first time somebody rewrote the sentence.
    pub marker: String,
    pub media_type: &'static str,
    pub bytes: Vec<u8>,
}

/// A paragraph pasted into the line, standing behind the marker written in its place.
///
/// Several lines of text in the box push everything else off the screen, and what they push off is
/// the reply the paste was about. So a paste of any length reads as one row, and the row says how
/// many lines are behind it.
///
/// The marker is the handle, as it is for a picture, and it is the only part of this a user can
/// see: deleting it is how the paste is taken back. Unlike a picture it is put back before the
/// prompt is sent, because here the marker stands for the words themselves rather than for
/// something travelling beside them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastedText {
    /// The text standing for it in the line, as `[Pasted text #1 +12 lines]`.
    pub marker: String,
    /// What was pasted, with its line endings already normalised.
    pub text: String,
}

/// What the last frame laid the transcript out to.
///
/// Written back after every draw, because none of it is knowable before one: the answers exist
/// only once the paragraph has been wrapped at the width it is being shown at. A key pressed next
/// is then answered against the frame the person is looking at, which is the one this describes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Laid {
    /// Columns the transcript was drawn in.
    pub width: u16,
    /// Rows it had room for.
    pub height: u16,
    /// Rows the whole of it came to.
    pub rows: u16,
    /// The row each prompt the person typed begins at, in the order they were typed.
    ///
    /// Empty unless the scroller is open, since working it out costs a wrap of every line and
    /// nothing at rest asks the question.
    pub prompts: Vec<u16>,
    /// The rows holding a search match, top to bottom.
    ///
    /// One entry per row rather than per match: the view moves to a row, and two hits on one row
    /// are one place to go.
    pub matches: Vec<u16>,
}

/// The scroller, while it is open.
///
/// Holds what the mode itself is doing and nothing else. Where the view is looking stays in the
/// field the wheel and the arrows move at rest, so opening the scroller and closing it again move
/// nothing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Scroller {
    /// What a finished search is looking for. Empty until one has been run.
    pub needle: String,
    /// A search being typed, before Enter runs it or Escape abandons it.
    pub typing: Option<String>,
    /// Whether the key list is up.
    pub help: bool,
    /// Which match the view is on, counted from zero.
    ///
    /// Clamped where it is read. A list laid out afresh can be shorter than the one this indexed,
    /// because a turn goes on writing underneath.
    pub at: usize,
}

/// Where `needle` occurs in `text`, as character ranges, left to right and never overlapping.
///
/// Literal, character for character. A needle is what somebody typed to find something they have
/// already seen, and a pattern language here would be an interpreter reached by a line typed over
/// text an attacker may have written, with a class of stalls behind it.
///
/// Case-insensitive while the needle is all lower case, exact from the moment it holds a capital,
/// which is the rule every editor with a search box already uses. Folding takes the first
/// character of a lowering so an offset counts the same in both strings.
pub fn matched(text: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let exact = needle.chars().any(char::is_uppercase);
    let fold = |c: char| {
        if exact {
            c
        } else {
            c.to_lowercase().next().unwrap_or(c)
        }
    };
    let hay: Vec<char> = text.chars().map(fold).collect();
    let pin: Vec<char> = needle.chars().map(fold).collect();

    // A forward scan that never goes back, so nothing it is pointed at can make it take long.
    let mut found = Vec::new();
    let mut at = 0;
    while at + pin.len() <= hay.len() {
        if hay[at..at + pin.len()] == pin[..] {
            found.push((at, at + pin.len()));
            at += pin.len();
        } else {
            at += 1;
        }
    }
    found
}

/// The delegate view, while it is open.
///
/// Two levels rather than one, because a turn has one delegate or nine. The list is the way in
/// where there are several, and one delegate's own lines are what somebody came to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watching {
    /// Which row this is about, by its place in the list: the delegates in the order they were
    /// spawned, then the commands in the order they ran.
    ///
    /// A position rather than a name, because it is also where the highlight sits in the list, and
    /// the two must not be able to disagree.
    pub at: usize,
    /// Whether the list is what is on the screen, rather than the row at `at`.
    pub listing: bool,
    /// Whether the list's highlight is on the session rather than on one of the rows.
    ///
    /// Kept beside `at` rather than folded into it so that `at` stays the row somebody was last
    /// reading: coming back into the list puts the highlight where they left it, and the way out
    /// is a row rather than a position nothing else can name.
    pub on_session: bool,
}

/// How the context currently stands in a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occupancy {
    /// No request has been measured yet.
    Unmeasured,
    /// Compaction shortened the conversation and the next turn has not measured yet.
    Compacted,
    /// Measured token count against the budget, and whether the budget was guessed.
    Measured {
        used: u64,
        budget: u64,
        guessed: bool,
    },
}

impl Occupancy {
    /// How full the context is, as a percentage, or `None` where nothing has been measured.
    pub fn percent(&self) -> Option<u64> {
        match *self {
            Occupancy::Measured { used, budget, .. } if used > 0 && budget > 0 => {
                Some((used.saturating_mul(100) / budget).min(100))
            }
            _ => None,
        }
    }
}

/// A checkpoint of session state captured before a turn begins, for `/undo`.
#[derive(Debug, Clone)]
pub struct TurnSnapshot {
    /// The conversation state (messages, references, context).
    pub conversation: bravebot_agent::conversation::Snapshot,
    /// Completed turns count before this turn.
    pub turns: usize,
    /// Cumulative tokens before this turn.
    pub tokens: u64,
    /// Spend by turn before this turn.
    pub spend: std::collections::BTreeMap<usize, u64>,
    /// Timing by turn before this turn.
    pub timing: std::collections::BTreeMap<usize, bravebot_agent::timing::Timing>,
    /// Trust map rules before this turn.
    pub trust: bravebot_core::trust::TrustStore,
    /// Trusted programs before this turn.
    pub programs: bravebot_core::programs::TrustedPrograms,
    /// Length of transcript entries before this turn.
    pub transcript_len: usize,
    /// Stored session title before this turn.
    pub title: String,
    /// Whether the session had already been written to disk before this turn.
    pub was_wrote: bool,
}

/// Everything the interface needs to draw itself.
#[derive(Debug)]
pub struct Session {
    pub transcript: Vec<Entry>,
    /// The line being typed.
    ///
    /// Private, with `caret`, because the two are one value: a caret is an offset into this string
    /// and nothing may shorten the string without moving it. Read it with [`Session::input`].
    input: String,
    /// Where the next keystroke lands, as a byte offset into `input`.
    ///
    /// A byte offset rather than a character index because every use of it is a slice of `input`,
    /// and it is kept on a character boundary by everything that moves it. Anything that replaces
    /// the line puts it at the end, which is where the line was left off.
    caret: usize,
    /// A line set aside to be typed again later, if there is one.
    ///
    /// Text alone, with no caret and no mode: what was put away is the words, and where the caret
    /// was in them is a fact about an edit that has finished. One slot rather than a stack, so the
    /// key that fills it and the key that empties it are the same key and neither has a depth to
    /// remember.
    stashed: Option<String>,
    /// Whether the line being typed is a command for the shell rather than a prompt for the model.
    ///
    /// Entered by typing `!` on an empty line and left by deleting back past it, so the `!` is a
    /// mode rather than a character: it is never part of `input`, and the command that runs is
    /// exactly what the user sees after the marker.
    pub shell: bool,
    /// Whether the list of keys is up.
    ///
    /// Like `shell`, the `?` that opens it is a mode rather than a character: it is never part of
    /// `input`, so putting the list up and taking it down again leaves nothing behind to delete.
    /// Opened only on an empty line, so it is never standing over a line it says nothing about.
    pub shortcuts: bool,
    /// Which style of editing the box does: the ordinary one, or vi's.
    ///
    /// A choice about the person rather than about the session, so it is read from `~/.bravebot` at
    /// startup and written back when one is made, the same as the model and the theme.
    editing: crate::vim::Editing,
    /// Which vi mode the box is in, where vi is the style.
    ///
    /// Every session opens in INSERT, where a typed character is a typed character. Opening in NORMAL
    /// would mean the first sentence somebody typed went nowhere, and a box that swallows what is
    /// typed into it is indistinguishable from one that has stopped working.
    ///
    /// Meaningless while the style is the ordinary one, and never read there. The two are separate
    /// fields because the mode outlives a trip through the ordinary style and back: somebody who
    /// turns vi editing off to paste something and on again is where they left off.
    mode: crate::vim::Mode,
    /// A vi instruction that arrived without everything it needs, waiting for one more key.
    ///
    /// `f` alone says to jump to a character nobody has named yet, so nothing happens until the next
    /// press names it. Held rather than acted on, and cleared by the press that completes or abandons
    /// it: a wait that outlived the pair would swallow letters typed later.
    half_typed: Option<crate::vim::Pending>,
    /// The last jump to a character, for the keys that repeat one.
    ///
    /// Remembered because `;` and `,` mean nothing on their own: they say "that again", and there is
    /// nothing else on the session that says what "that" was.
    last_find: Option<crate::vim::Find>,
    /// What the last yank or delete took, for the keys that put it back.
    ///
    /// Vi's unnamed register, and the only one: the named ones are a filing system, and a box holding
    /// one line of thought has nothing to file. Not the system clipboard either, which Ctrl-V owns and
    /// which a person shares with every other window they have open.
    register: Option<Yanked>,
    /// The last change, for the key that does it again.
    ///
    /// The instruction rather than what it produced, so `.` acts at the caret wherever that now is.
    /// That is the whole of why the key is worth having: the change is repeated somewhere else.
    last_change: Option<(crate::vim::Operator, crate::vim::Extent)>,
    /// The line as it stood before the last change, for the key that puts it back.
    ///
    /// One step rather than a stack, on the same footing as the stash: the key that undoes and the
    /// keystroke that will be regretted are one press apart, and a depth is a thing to remember.
    before_last_change: Option<(String, usize)>,
    pub status: Status,
    /// Whether the audit trail is shown alongside replies.
    pub show_trail: bool,
    /// Scroll offset from the bottom, in lines.
    pub scroll: u16,
    /// The scroller, while it is open.
    ///
    /// `None` at rest, which is what every key in the box is answered against: the mode is the
    /// one thing that decides whether a letter is a letter or a movement.
    scroller: Option<Scroller>,
    /// The delegate view, while it is open.
    ///
    /// `None` at rest, on the same footing as the scroller: what decides whether a letter is a
    /// letter or a movement is the mode, never what happens to be on the screen.
    watching: Option<Watching>,
    /// What each command this session ran printed, oldest first.
    ///
    /// Held here rather than on the entry that ran it, because the view lists them across the
    /// whole session the way it lists delegates, and an entry is the wrong place to look for the
    /// third command when the second one scrolled away.
    outputs: Vec<Output>,
    /// Where the turn's own view was when somebody went to look at a delegate.
    ///
    /// Held rather than recomputed, so coming back puts them where they were reading instead of
    /// at the end of a transcript that has moved on while they were away.
    held_view: Option<u16>,
    /// The search over the prompt history, while it is open.
    ///
    /// `None` at rest, on the same footing as the scroller: a mode is what decides whether a
    /// letter typed is a letter in the box or one narrowing a list.
    history_search: Option<crate::history_search::Search>,
    /// What the last frame laid the transcript out to.
    pub laid: Laid,
    /// Confinement in force, reported so the user knows what they have.
    pub confinement: String,
    /// How much this session asks before it acts, which one key cycles.
    ///
    /// Not persisted, like `shell` and unlike the trust map: a mode is a standing answer somebody
    /// gave while watching a particular piece of work, and a resumed session is a different sitting.
    /// Coming back tomorrow into a session that had stopped asking about writes, with nothing on
    /// screen having been chosen today, is the wrong way for this to be wrong.
    permission_mode: bravebot_agent::PermissionMode,
    /// Whether `--dangerously-skip-permissions` was given, which is what puts the fourth rung on the
    /// ladder above. Fixed for the session: it comes from the command line.
    bypass_available: bool,
    /// What the configuration says about the tier, drawn beside the confinement on the opening
    /// screen.
    ///
    /// The configuration rather than the credentials, because a stored batch may be expired or for
    /// the wrong environment, so its presence would not settle the tier. See
    /// [`crate::status::configured_tier`]. What was actually spent is settled by the first turn.
    pub tier: String,
    /// How many turns have been submitted, which picks the indicator's word.
    pub turns: usize,
    /// Tokens spent across the whole session.
    pub tokens: u64,
    /// What each turn cost, by turn number.
    ///
    /// The session total answers "what has this cost me"; this answers "where did it go", which is
    /// the question when one turn spent most of it. A total alone cannot distinguish a session of
    /// twenty even turns from one turn that ran away, and those want different fixes.
    spend: std::collections::BTreeMap<usize, u64>,
    /// Where each turn's wall clock went, by turn number.
    ///
    /// Kept beside [`Session::spend`] and written down with it. Tokens say what a turn cost the
    /// endpoint; this says what it cost the person, split so the two things a person can act on are
    /// separable from the one they cannot: what was spent waiting on them, and what was spent
    /// waiting on the model.
    timing: std::collections::BTreeMap<usize, bravebot_agent::timing::Timing>,
    /// How large the last request was, and the budget it is compacted at.
    ///
    /// Not the same figure as [`Session::tokens`], which adds every round of every turn together
    /// and so says what the session has cost. This says how full the context is now.
    occupancy: Occupancy,
    /// What the last turn actually asked for, and what the server answered with.
    ///
    /// `None` until a turn has run, which is the honest reading: whether premium is in use is not a
    /// fact about the configuration, it is a fact about a request, and before the first one there is
    /// nothing to report. Reporting the build's premium host instead said "premium configured" for a
    /// whole session that never spent a credential.
    ///
    /// Both halves, because the interesting case is when they differ: the endpoint answers a model
    /// name it will not serve by substituting a weaker one rather than by failing, so a session can
    /// ask for Opus all day and be answered by something else with nothing said.
    served: Option<(Option<String>, String)>,
    /// Whether the two halves of [`Session::served`] are names from one roster.
    ///
    /// False where the request named an opaque handle standing for a model rather than a model, since
    /// the reply then names something different every time and nothing is wrong. Recorded by the
    /// caller, which knows which backend answered, rather than inferred from how a name is spelled.
    served_names_are_comparable: bool,
    /// Whether the last turn actually spent a subscription credential.
    ///
    /// `None` until a turn has run. Observed rather than derived from the configuration, which is
    /// the same reasoning as [`Session::served`]: whether premium is in use is a fact about a
    /// request, and every build knows a premium host whether or not one is ever reached.
    premium: Option<bool>,
    /// Prompts already sent, for recall with the arrow keys.
    pub history: crate::history::History,
    /// What the mouse is sweeping over, or what it last swept over.
    ///
    /// Kept after the button comes up, so a user can see what they copied rather than watching
    /// it vanish at the moment it is taken.
    pub selection: Option<crate::select::Selection>,
    /// How much the last copy took, until the next thing happens.
    pub copied: Option<usize>,
    /// What the turn that just finished cost, until the next one starts.
    ///
    /// The spinner going out is how the end of a turn used to be announced, and an announcement
    /// made by something disappearing is one nobody reads. It matters most for the turn that ends
    /// on a sentence like "now let me look at the dispatch code": the model asked for no tool, so
    /// the turn was over, and the only thing that said so was a line that was no longer there.
    ///
    /// `None` before the first turn, so a fresh session says nothing rather than claiming a turn
    /// that has not happened.
    pub finished: Option<Finished>,
    /// Whether the last press took a line out of the box rather than ending the session.
    ///
    /// The hint saying which key ends it hangs on this. It lives for exactly one press, because it
    /// answers the press just made and the next press is the answer to it.
    pub cleared_by_interrupt: bool,
    /// Whether there was a picture on the clipboard when it was last looked at.
    ///
    /// Only ever a hint on screen, so a stale answer costs a line that is briefly wrong and nothing
    /// else. Looked at when the terminal regains focus, which is when somebody has just been
    /// somewhere else copying something, and cleared by a paste, since carrying on saying it after
    /// the picture is in the prompt is nagging.
    pub image_on_clipboard: bool,
    /// Tokens the model has written during the turn in flight.
    ///
    /// Reset when a turn starts, since it measures the reply being written now. The session total
    /// lives in `tokens` and accumulates instead.
    pub written: u64,
    /// The task list for the turn in flight, as the model last reported it.
    ///
    /// Already shaped and released: these rows came out of the kernel's render gate, so drawing
    /// them decides nothing and needs no label. Cleared when a turn starts, so one turn's plan
    /// never appears beneath another's work.
    pub todos: Vec<bravebot_core::todo::Row>,
    /// What the turn in flight is waiting on, when it is waiting on the model.
    ///
    /// Cleared between turns. `None` before the first request goes out, which is the only
    /// moment the generic word is all there is to say.
    pub phase: Option<Phase>,
    /// What the files the last turn wrote to held before it wrote to them.
    ///
    /// Taken off the workspace when the turn ends, so `/undo` can put them back.
    pub last_turn_backups: Vec<bravebot_agent::workspace::Backup>,
    /// The state before the most recent turn.
    ///
    /// Captured in memory right before a turn starts, and kept here so `/undo` can rebuild
    /// the session to match it.
    pub previous_turn: Option<TurnSnapshot>,
    /// The tool call in flight, if one is.
    ///
    /// Also in the transcript, where it stays. Kept here as well because the indicator needs
    /// to name it, and scanning back through the transcript for the tail would be a worse way
    /// to answer a question the session already knows the answer to.
    pub running: Option<Activity>,
    /// Prompts typed and sent while a turn was running, in the order they were typed.
    ///
    /// Not in the transcript: they have not happened. They are drawn under the box as waiting,
    /// and each moves into the transcript at the moment it reaches the planner, whether that is
    /// inside the running turn or as a turn of its own.
    pub queued: Vec<Queued>,
    /// The loop repeating a prompt, where the person started one.
    ///
    /// Private, because every part of it has to move together: a tick is dispatched, the turn
    /// runs, and only then is the next one armed. A field anybody could set would let the three
    /// disagree, and the disagreement a caller would reach for first is arming a tick while one
    /// is still running.
    ///
    /// Not in [`crate::sessions::Standing`], so nothing about it is written down. A schedule that
    /// outlived the session that set it would start sending prompts at somebody who resumed a
    /// conversation to read it.
    looping: Option<crate::loops::Running>,
    /// The same prompts, resolved, for the turn in flight to take between rounds.
    ///
    /// Shared with the worker rather than sent down a channel, because a queued prompt can be
    /// taken back: a line already posted into a channel is gone, and Up would appear to retrieve
    /// a prompt that then arrived anyway. Both ends holding one buffer makes taking it back mean
    /// what it says.
    ///
    /// Mirrors `queued`, minus whatever the turn has already taken.
    pending: crate::remote_confirm::Interjections,
    /// The reply the model is writing right now, as far as it has got.
    ///
    /// Not in the transcript, because it is not a thing that happened yet: it is drawn at the
    /// tail and replaced by the entry the round produces. Keeping it apart is what makes that
    /// handover free of a duplicate, and it is why a session written to disk holds finished
    /// turns rather than a half-finished sentence.
    ///
    /// Held as it arrived. What is drawn from it is [`Session::reply_so_far`], since a model
    /// that has nowhere else to put its working writes it in here.
    streaming: String,
    /// Whose work the reports arriving now describe, where it is a delegate's.
    ///
    /// Set by the driver and never worked out here. Delegates report alongside the turn and
    /// alongside each other, so where a line arrived in the sequence says nothing about whose it
    /// is. Nothing outlives a turn here: the driver clears it when a delegate finishes, and a
    /// delegate cannot outlive the turn that started it.
    attributed_to: Option<bravebot_agent::report::DelegateId>,
    /// Answers the user has already given this session, keyed by the question.
    ///
    /// A repeated question is answered from here rather than put to them again, since a planner
    /// that loops back over the same decision should not make the user restate it. Kept in the
    /// interface rather than the kernel: it is a convenience for the person, not a rule about
    /// labels, and the key is trusted text by the time it reaches here.
    pub answers: Vec<(String, bravebot_core::ask::Answer)>,
    /// Pictures pasted into the line being typed, each with the text standing in for it.
    ///
    /// The marker is the handle. A paste writes `[Image #1]` where the caret was and the picture
    /// travels wherever that text travels, so deleting the marker is how a picture is taken back
    /// and recalling an older prompt leaves none of them behind. Nothing here is pruned as the line
    /// is edited: the line is the record, and this is read against it whenever the answer matters.
    pasted: Vec<AttachedImage>,
    /// The pictures the line carried when it was sent.
    ///
    /// Settled by [`Session::submit`] alongside `sent`, and for the same reason: that is the
    /// moment the line stops changing.
    sent_pasted: Vec<AttachedImage>,
    /// Paragraphs pasted into the line, each with the marker standing in for it.
    ///
    /// Kept for the life of the session rather than settled and cleared the way pictures are. A
    /// marker with nothing behind it costs a picture and leaves the words; here the marker *is* the
    /// words, so a prompt recalled out of the history with one in it would send the placeholder in
    /// place of everything the user pasted, and they would have no way to tell. Numbers are never
    /// reused, so a marker means one thing for as long as the session lasts.
    pasted_text: Vec<PastedText>,
    /// Whether history is written to disk.
    ///
    /// Off by default so constructing a session does no I/O: a test would otherwise read and
    /// write the developer's own history, and one that ran twice would see the first run's
    /// prompts. The real session turns it on with [`Session::with_stored_history`].
    persist: bool,
    /// Notes already said once, so a standing condition is not repeated every turn.
    ///
    /// Skills and standing instructions are looked for afresh each turn, which is what lets one
    /// written mid-session take effect. The reasons a file was left out therefore recur every
    /// turn as well, and saying them each time would bury the work in a condition the user
    /// already knows about and cannot fix from here.
    said: Vec<String>,
    /// When the turn in flight started. `None` when idle.
    ///
    /// An `Instant` rather than a stored elapsed value so the display advances between redraws
    /// without anything having to tick it.
    started: Option<Instant>,
    /// The model the user chose, or `None` to use the configured default.
    ///
    /// Read from `~/.bravebot` at startup and rewritten when `/model` picks one, so the choice outlives
    /// the session that made it and applies in every directory.
    model: Option<String>,
    /// How hard to think, or `None` to leave the service its own default.
    ///
    /// Read from `~/.bravebot` at startup and rewritten when `/effort` picks one, the same as the
    /// model: how hard to think is a preference about the work rather than a property of a
    /// checkout.
    effort: Option<Effort>,
    /// Whether the model in force reads the effort level, as its roster row stated.
    ///
    /// True until a listing says otherwise, so a session that has never reached one sends what the
    /// person asked for rather than withholding it on a fact nobody established.
    model_reads_effort: bool,
    /// Which offered command is under the cursor while one is being typed.
    ///
    /// An index into what [`Session::offered`] returns for the current input rather than a copy of
    /// the list, because the list is a function of the input and keeping a second copy in step with
    /// it is the way the two come to disagree. Clamped when it is read, since typing another letter
    /// can shorten the list under a cursor that was further down.
    completion: usize,
    /// The directory a file reference is completed against.
    ///
    /// Empty by default so constructing a session reads no directory: a test would otherwise offer
    /// whatever happened to be in the process's working directory. The real session names it with
    /// [`Session::in_workspace`].
    workspace: std::path::PathBuf,
    /// Files dropped on the box, by the marker standing for each in the line.
    ///
    /// Kept until the line is sent, and read back out of the line at that point rather than sent
    /// wholesale: deleting a marker is how a user takes an attachment off, and it has to be, since
    /// the marker is the only thing they can see to delete.
    attached: Vec<Attached>,
    /// The attachments the line carried when it was sent.
    ///
    /// Settled by [`Session::submit`], because that is the moment the line stops changing, and
    /// read by the caller building the task after the box has already been cleared.
    sent: Vec<Attached>,
    /// How many attachments this session has made, so a marker is never reused.
    ///
    /// Counts up rather than indexing the list. Renumbering the rest when one is deleted would
    /// change the marker sitting in the line the user is looking at.
    attachments_made: usize,
}

impl Session {
    pub fn new(confinement: impl Into<String>) -> Self {
        Self {
            transcript: Vec::new(),
            input: String::new(),
            caret: 0,
            stashed: None,
            shell: false,
            shortcuts: false,
            // The box everybody has, until a settings file or a choice says otherwise. A session
            // constructed by a test reads nothing from disk and edits the ordinary way.
            editing: crate::vim::Editing::default(),
            mode: crate::vim::Mode::default(),
            half_typed: None,
            last_find: None,
            register: None,
            last_change: None,
            before_last_change: None,
            status: Status::Idle,
            show_trail: false,
            scroll: 0,
            scroller: None,
            watching: None,
            outputs: Vec::new(),
            held_view: None,
            history_search: None,
            laid: Laid::default(),
            confinement: confinement.into(),
            // Asking, which is what a session has always done. `allowing_bypass` moves it, and is
            // the only thing that can: the flag is the record that somebody accepted the cost.
            permission_mode: bravebot_agent::PermissionMode::default(),
            bypass_available: false,
            // The free tier until a caller says otherwise, which is what a build with no premium
            // host has and what a test that does not care about tiers should see.
            tier: t!(status_free_tier).to_string(),
            turns: 0,
            tokens: 0,
            spend: std::collections::BTreeMap::new(),
            timing: std::collections::BTreeMap::new(),
            occupancy: Occupancy::Unmeasured,
            served: None,
            // Nothing has been served, so nothing has been compared. Set by the first turn.
            served_names_are_comparable: true,
            premium: None,
            history: crate::history::History::new(),
            selection: None,
            copied: None,
            finished: None,
            cleared_by_interrupt: false,
            image_on_clipboard: false,
            written: 0,
            todos: Vec::new(),
            phase: None,
            running: None,
            queued: Vec::new(),
            looping: None,
            last_turn_backups: Vec::new(),
            previous_turn: None,
            pending: crate::remote_confirm::Interjections::new(),
            streaming: String::new(),
            attributed_to: None,
            answers: Vec::new(),
            pasted: Vec::new(),
            sent_pasted: Vec::new(),
            pasted_text: Vec::new(),
            persist: false,
            said: Vec::new(),
            started: None,
            model: None,
            effort: None,
            model_reads_effort: true,
            completion: 0,
            workspace: std::path::PathBuf::new(),
            attached: Vec::new(),
            sent: Vec::new(),
            attachments_made: 0,
        }
    }

    /// Complete file references against this directory.
    ///
    /// Separate from [`Session::new`] so listing a directory is a deliberate choice at one call
    /// site rather than a side effect of constructing a session.
    pub fn in_workspace(mut self, root: impl Into<std::path::PathBuf>) -> Self {
        self.workspace = root.into();
        self
    }

    /// Complete against somewhere else from now on, the working directory having moved.
    ///
    /// The box has to follow, and not only for convenience: `@` completes to a path that is then
    /// resolved against the working directory, so a list still drawn from the old one would offer
    /// names that name nothing, or worse, name a different file of the same name.
    pub fn now_in_workspace(&mut self, root: impl Into<std::path::PathBuf>) {
        self.workspace = root.into();
    }

    /// Say what the configuration allows, for the opening screen to draw.
    pub fn on_tier(mut self, tier: impl Into<String>) -> Self {
        self.tier = tier.into();
        self
    }

    /// Open the session in bypass, because `--dangerously-skip-permissions` asked for it.
    ///
    /// Both at once, and they belong together: the flag puts the fourth rung on the ladder *and*
    /// starts the session on it. Honouring only the first would make the flag do nothing a person
    /// could see, and disagree with what the same flag does to a one-shot run.
    pub fn allowing_bypass(mut self) -> Self {
        self.bypass_available = true;
        self.permission_mode = bravebot_agent::PermissionMode::Bypass;
        self
    }

    /// How much this session asks before it acts.
    pub fn permission_mode(&self) -> bravebot_agent::PermissionMode {
        self.permission_mode
    }

    /// Move to the next mode, and say nothing: the line under the box is the answer.
    ///
    /// A note in the transcript would be a running commentary on a key somebody is pressing to see
    /// what the modes are, and the one place a mode has to be legible is while it is in force.
    pub fn cycle_permission_mode(&mut self) {
        self.permission_mode = self.permission_mode.cycle(self.bypass_available);
    }

    /// Load history from disk and keep writing to it.
    ///
    /// Separate from [`Session::new`] so persistence is a deliberate choice at one call site
    /// rather than a side effect of constructing a session.
    ///
    /// What comes back is not trusted: the file can be edited, so a recalled prompt goes into the
    /// input box for the user to read and submit. That keystroke is what makes it trusted, exactly
    /// as typing it would have been.
    pub fn with_stored_history(mut self) -> Self {
        self.history = crate::history::History::from_entries(crate::store::load_history());
        self.model = crate::store::load_model();
        self.effort = crate::store::load_effort();
        self.persist = true;
        self
    }

    /// The model to request, or `None` to use the configured default.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Record the model the user picked, keeping it for later sessions.
    ///
    /// Written through to disk only for a session that persists, which is the same rule history
    /// follows and for the same reason: a test must not rewrite the developer's own choice.
    pub fn choose_model(&mut self, model: impl Into<String>) {
        let model = model.into();
        if self.persist {
            crate::store::save_model(&model);
        }
        self.model = Some(model);
    }

    /// How hard to think, or `None` to leave the service its own default.
    ///
    /// What the person chose, which is not always what a request carries. For reporting the choice
    /// and for opening the picker on it.
    pub fn effort(&self) -> Option<Effort> {
        self.effort
    }

    /// The level a turn will actually send, which is nothing where the model does not read one.
    ///
    /// Sending a level to a model whose roster row says it reads none is a field that is dropped at
    /// the far end, so the choice is kept and the request is not given it.
    pub fn effort_in_force(&self) -> Option<Effort> {
        self.effort.filter(|_| self.model_reads_effort)
    }

    /// Whether the model in force reads a level at all.
    ///
    /// For saying so where it does not: a level reported as in force while the service discards it
    /// is the interface telling somebody they bought something they did not.
    pub fn model_reads_effort(&self) -> bool {
        self.model_reads_effort
    }

    /// Record what the roster said about the model now in force.
    pub fn note_model_reads_effort(&mut self, reads: bool) {
        self.model_reads_effort = reads;
    }

    /// Record the level the user picked, keeping it for later sessions.
    ///
    /// `None` is a choice too: it puts the session back to sending no level at all, which is what
    /// somebody who has never picked one is already doing. Written through to disk only for a
    /// session that persists, the same rule the model follows and for the same reason.
    pub fn choose_effort(&mut self, effort: Option<Effort>) {
        if self.persist {
            crate::store::save_effort(effort);
        }
        self.effort = effort;
    }

    /// How long the turn in flight has been running, or zero when idle.
    pub fn elapsed(&self) -> Duration {
        self.started.map(|t| t.elapsed()).unwrap_or_default()
    }

    /// What the indicator should call what is happening, most specific first.
    ///
    /// A call in flight is the most immediate answer, then the task the model says it is on,
    /// then the phase it is waiting in. `None` only before the first request goes out, when
    /// there is genuinely nothing to say yet and the turn's own word is all there is.
    fn what_is_happening(&self) -> Option<String> {
        // Only the phases that say something a person cannot see elsewhere. Planning is the
        // first call, before any line has appeared, and reconnecting is a pause that looks
        // exactly like thinking and is not: nothing is being worked out and what the model had
        // written has been thrown away.
        //
        // Compacting is one of them too: the request is being summarised, which takes as long as
        // a round and produces nothing to look at, so without a word for it the session looks
        // stuck at the moment it is doing the most.
        //
        // Thinking is not one of them, and neither is the call in flight or the task in hand:
        // both of those are already on their own lines in the transcript above, and repeating
        // the running call here left the spinner reading "Isolated processor(index.html,
        // server.py)…", which is a strange thing for a word beside a spinner to be. What that
        // word is for is showing that the session is alive while the answer takes its time.
        match self.phase {
            Some(phase @ (Phase::Planning | Phase::Reconnecting | Phase::Compacting)) => {
                Some(phase.word().to_string())
            }
            _ => None,
        }
    }

    /// The indicator to show, or `None` when no turn is running.
    ///
    /// Named after whatever is most specific about the moment, so the line answers the question
    /// a waiting user actually has. Falls back to the turn's own word only before anything has
    /// happened at all.
    pub fn indicator(&self) -> Option<crate::indicator::Indicator> {
        (self.status == Status::Working).then(|| {
            let base = crate::indicator::Indicator::new(
                self.turns.saturating_sub(1),
                self.elapsed(),
                self.tokens,
            );
            let base = base.writing(self.written);
            match self.what_is_happening() {
                Some(what) => base.labelled(what),
                None => base,
            }
        })
    }

    /// Record the task list the turn just reported.
    pub fn set_todos(&mut self, rows: Vec<bravebot_core::todo::Row>) {
        self.todos = rows;
    }

    /// Take on what an earlier session spent.
    ///
    /// The counter answers "what has this cost me", and that answer does not become smaller
    /// because the process restarted. Set rather than added to: this is a session being picked
    /// up, not a second one being merged into it.
    pub fn restore_spend(&mut self, tokens: u64, by_turn: std::collections::BTreeMap<usize, u64>) {
        self.tokens = tokens;
        self.spend = by_turn;
    }

    /// Take on how an earlier session's turns spent their time.
    ///
    /// Separate from [`Session::restore_spend`] because a record written before timing was kept has
    /// the one and not the other, and a resume must not have to choose between restoring both or
    /// neither.
    pub fn restore_timing(
        &mut self,
        by_turn: std::collections::BTreeMap<usize, bravebot_agent::timing::Timing>,
    ) {
        self.timing = by_turn;
    }

    /// What each turn cost, by turn number, for writing the session down.
    pub fn spend_by_turn(&self) -> &std::collections::BTreeMap<usize, u64> {
        &self.spend
    }

    /// Where each turn's wall clock went, by turn number, for writing the session down.
    pub fn timing_by_turn(
        &self,
    ) -> &std::collections::BTreeMap<usize, bravebot_agent::timing::Timing> {
        &self.timing
    }

    /// Every turn's timing added together.
    pub fn timing_total(&self) -> bravebot_agent::timing::Timing {
        let mut total = bravebot_agent::timing::Timing::default();
        for timing in self.timing.values() {
            total.add(*timing);
        }
        total
    }

    /// Begin again with nothing behind you.
    ///
    /// Everything about the exchange goes: the transcript, the turn count, and what it spent. The
    /// trust map is not held here and goes with it, along with the directories opened under it; the
    /// caller asks the trust question again, because this begins a session and every session is
    /// asked.
    ///
    /// What stays is what belongs to the user rather than to the session: the model, the prompt
    /// history, and the confinement in force. Re-answering those would be the interface forgetting
    /// something it was told once, and none of them is a permission over the workspace.
    ///
    /// Deliberately not touching the input line, so a prompt half-typed when the user cleared is
    /// still there to send.
    /// Give up the rewind the last turn left available.
    ///
    /// A snapshot describes the session as it stood before the last turn, so anything that
    /// changes the session outside a turn leaves it describing something else. Rewinding to a
    /// stale snapshot would undo that change as well, silently and under a line saying one turn
    /// was rewound.
    pub fn close_rewind_window(&mut self) {
        self.previous_turn = None;
        self.last_turn_backups.clear();
    }

    pub fn clear(&mut self) {
        self.transcript.clear();
        self.turns = 0;
        self.tokens = 0;
        self.spend.clear();
        self.timing.clear();
        self.occupancy = Occupancy::Unmeasured;
        self.written = 0;
        self.todos.clear();
        self.phase = None;
        self.running = None;
        self.started = None;
        self.scroll = 0;
        self.selection = None;
        self.copied = None;
        self.finished = None;
        self.close_rewind_window();
        // A standing condition is worth saying once per session, and this is now a new one: the
        // reason a skill was left out applies to the next turn as much as it did to the last.
        self.said.clear();
        // The loop goes with the conversation it was started in. A schedule surviving into a
        // session that knows nothing about it would send a prompt whose context has been thrown
        // away, which is neither what was asked for nor recognisable as a mistake.
        self.looping = None;
        // The delegates went with the transcript that held them, so the mode standing over one
        // is standing over nothing. What the commands printed is kept beside the transcript rather
        // than in it, so it is dropped here by name: a conversation nobody remembers leaving its
        // commands openable is the one case the view could show work from a session that is gone.
        self.outputs.clear();
        self.watching = None;
        self.held_view = None;
    }

    /// The task list each turn finished with, by turn number, for writing the session down.
    ///
    /// Read back off the transcript rather than kept in a second place, because the transcript is
    /// already where a finished turn's list lives: `complete` moves it there so the scrollback
    /// shows what each turn set out to do. Turn numbers are counted the way `replay` counts them,
    /// so a list written under turn three comes back under turn three.
    pub fn todos_by_turn(
        &self,
    ) -> std::collections::BTreeMap<usize, Vec<bravebot_core::todo::Row>> {
        let mut by_turn = std::collections::BTreeMap::new();
        let mut turn = 0;
        for entry in &self.transcript {
            if entry.speaker == Speaker::User {
                turn += 1;
            }
            if turn > 0 && !entry.todos.is_empty() {
                by_turn.insert(turn, entry.todos.clone());
            }
        }
        by_turn
    }

    /// Record how much the model has written so far in the turn in flight.
    pub fn set_written(&mut self, written: u64) {
        self.written = written;
    }

    /// Record what the turn is waiting on.
    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = Some(phase);
        // A phase is announced once at the top of every round and again when a request is being
        // sent afresh. Either way what was on the screen belongs to a reply that is over or to
        // one that has been thrown away, so the tail starts empty.
        self.streaming.clear();
    }

    /// Add what the model has written since the last frame to the reply taking shape.
    ///
    /// Empty text is dropped here rather than by the turn, for the same reason narration is:
    /// this side may look at released text, and the turn may not.
    pub fn streaming(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // A delegate's half-written sentence is not the turn's. Drawn under the turn's own reply
        // it would read as the planner writing something it never wrote, and there is one model
        // writing at a time.
        if self.attributed_to.is_some() {
            return;
        }
        self.streaming.push_str(text);
        self.scroll = 0;
    }

    /// The part of the reply taking shape that is meant for the person watching.
    ///
    /// Not the whole of what arrived. A model with no channel of its own for its working writes
    /// it into the reply, and a person waiting on an answer is not waiting on that. See
    /// [`crate::reasoning`].
    pub fn reply_so_far(&self) -> &str {
        crate::reasoning::spoken_so_far(&self.streaming)
    }

    /// Record what the model said on its way to the next tool call.
    ///
    /// Empty text is dropped here rather than by the turn, which cannot look at it to decide.
    /// This side may: the text has been released, and a blank line in a transcript is a
    /// presentation question.
    pub fn narrate(&mut self, text: impl Into<String>) {
        // Cleared first, and whatever the text turns out to be. This is the same words the tail
        // has been showing, now on their way into the transcript, so leaving the tail up would
        // draw them twice; and a round that said nothing has nothing to leave up either.
        self.streaming.clear();
        let text = text.into();
        let text = crate::reasoning::spoken(&text);
        if text.trim().is_empty() {
            return;
        }
        // A delegate says a great deal on its way to an answer and none of it is the turn's. What
        // it concluded arrives as the report, which is the sentence its block ends on.
        if self.attributed_to.is_some() {
            return;
        }
        self.transcript.push(Entry::assistant(text, Vec::new()));
    }

    /// Whose work the reports that follow are, as the driver said.
    ///
    /// The one thing that decides where a line lands. Delegates work alongside the turn and
    /// alongside each other, so the lines arrive interleaved: nothing here reads a line to work
    /// out whose it was, because a line is prose a model had a hand in.
    pub fn reporting_for(&mut self, delegate: Option<bravebot_agent::report::DelegateId>) {
        self.attributed_to = delegate;
    }

    /// Where a report lands: under the delegate whose work it is, or in the turn's transcript.
    fn working_lines(&mut self) -> &mut Vec<Entry> {
        match self.attributed_to.and_then(|id| self.at(id)) {
            Some(at) => {
                &mut self.transcript[at]
                    .delegate
                    .as_mut()
                    .expect("a delegate entry holds its delegate")
                    .lines
            }
            None => &mut self.transcript,
        }
    }

    /// Where a delegate's block is, by the number the driver gave it.
    ///
    /// Searched from the end, because the one being reported on is almost always the one most
    /// recently started.
    fn at(&self, id: bravebot_agent::report::DelegateId) -> Option<usize> {
        self.transcript
            .iter()
            .rposition(|entry| entry.delegate.as_ref().is_some_and(|held| held.id == id))
    }

    /// A delegate has begun, drawn where the call that started it happened.
    pub fn delegate_started(&mut self, delegation: bravebot_agent::report::Delegation) {
        self.scroll = 0;
        let mut entry = Entry::system("");
        entry.speaker = Speaker::Delegate;
        entry.delegate = Some(Delegate {
            id: delegation.id,
            kind: delegation.kind,
            task: delegation.task,
            lines: Vec::new(),
            calls: 0,
            note: None,
            reported: None,
            failed: false,
        });
        self.transcript.push(entry);
    }

    /// One delegate has finished, with what the turn was told about it and what it reported.
    ///
    /// Its block collapses to those two: what it did is behind it, and what it concluded is the
    /// whole of what anybody acts on. Named rather than taken to be whichever was working,
    /// because the one that finishes is not the one that started last.
    pub fn delegate_finished(
        &mut self,
        id: bravebot_agent::report::DelegateId,
        note: String,
        failed: bool,
        reported: Option<Reported>,
    ) {
        if self.attributed_to == Some(id) {
            self.attributed_to = None;
        }
        if let Some(at) = self.at(id)
            && let Some(delegate) = self.transcript[at].delegate.as_mut()
        {
            delegate.note = Some(note);
            delegate.reported = reported;
            delegate.failed = failed;
        }
    }

    /// Every delegate this session has started, oldest first.
    pub fn delegates(&self) -> Vec<&Delegate> {
        self.transcript
            .iter()
            .filter_map(|entry| entry.delegate.as_ref())
            .collect()
    }

    /// What a command printed, kept for the view.
    ///
    /// Appended rather than attached to a line, so the third command is where the view expects it
    /// once the second one has scrolled away.
    pub fn command_printed(&mut self, printed: Printed) {
        self.outputs.push(Output {
            command: printed.command,
            lines: printed.lines,
            total: printed.total,
            read_by_the_planner: printed.read_by_the_planner,
            outcome: printed.outcome,
        });
    }

    /// Every command this session ran, oldest first.
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// Everything the view can open, in the order the list draws it.
    ///
    /// The delegates first and the commands after them, so a row's place does not move when the
    /// next command runs. Both are work that happened outside the transcript, which is what the
    /// view is for.
    pub fn watchable(&self) -> Vec<Watched<'_>> {
        self.delegates()
            .into_iter()
            .map(Watched::Delegate)
            .chain(self.outputs.iter().map(Watched::Output))
            .collect()
    }

    /// Open the view: on the list where there are several rows, and on the one where there is one.
    ///
    /// `false` where there is nothing to look at, which leaves the key doing nothing at all. A
    /// mode that opened on an empty screen would be worse than a key that did not answer.
    pub fn watch(&mut self) -> bool {
        let rows = self.watchable();
        let Some(last) = rows.len().checked_sub(1) else {
            return false;
        };
        // The delegate working, or the most recent row where none is. Somebody pressing the key
        // while something is happening means that one, and there is nothing else it could mean
        // when nothing is.
        let at = rows
            .iter()
            .rposition(|row| matches!(row, Watched::Delegate(delegate) if delegate.is_running()))
            .unwrap_or(last);
        let listing = last > 0;
        self.held_view = Some(self.scroll);
        self.scroll = 0;
        self.watching = Some(Watching {
            at,
            listing,
            on_session: false,
        });
        true
    }

    /// Close it, putting the turn's own view back where it was left.
    ///
    /// `false` where it was not open, so a key can tell whether it was the one that closed
    /// something from whether it has still to be answered by the ladder below.
    pub fn stop_watching(&mut self) -> bool {
        if self.watching.take().is_none() {
            return false;
        }
        self.scroll = self.held_view.take().unwrap_or(0);
        true
    }

    /// The delegate view, while it is open.
    pub fn watching(&self) -> Option<Watching> {
        self.watching
    }

    /// Whether one row's own lines are what is on the screen.
    pub fn watching_a_delegate(&self) -> bool {
        self.watching.is_some_and(|watching| !watching.listing)
    }

    /// Whether the list is what is on the screen.
    pub fn listing_delegates(&self) -> bool {
        self.watching.is_some_and(|watching| watching.listing)
    }

    /// The row the view is on, whether it is open or highlighted in the list.
    ///
    /// `None` where the list's highlight is on the session, which is a row and not one of these.
    pub fn watched(&self) -> Option<Watched<'_>> {
        let watching = self.watching.filter(|watching| !watching.on_session)?;
        self.watchable().get(watching.at).copied()
    }

    /// The delegate the view is on, where the row it is on is a delegate.
    pub fn watched_delegate(&self) -> Option<&Delegate> {
        match self.watched() {
            Some(Watched::Delegate(delegate)) => Some(delegate),
            _ => None,
        }
    }

    /// What a command printed, where the row the view is on is a command.
    pub fn watched_output(&self) -> Option<&Output> {
        match self.watched() {
            Some(Watched::Output(output)) => Some(output),
            _ => None,
        }
    }

    /// Whether the list's highlight is on the session rather than on one of the delegates.
    pub fn listing_on_the_session(&self) -> bool {
        self.watching
            .is_some_and(|watching| watching.listing && watching.on_session)
    }

    /// Where the highlight sits among the list's rows, the session being the first of them.
    pub fn list_highlight(&self) -> usize {
        match self.watching {
            Some(watching) if !watching.on_session => watching.at + 1,
            _ => 0,
        }
    }

    /// Open the delegate the list is on.
    pub fn open_watched(&mut self) {
        if let Some(watching) = &mut self.watching {
            watching.listing = false;
            self.scroll = 0;
        }
    }

    /// Go back to the list from one row's lines.
    ///
    /// `false` where there is only one row: there is no list behind it, and the key that would
    /// have gone back is the key that closes.
    pub fn list_delegates(&mut self) -> bool {
        if self.watchable().len() < 2 {
            return false;
        }
        match &mut self.watching {
            Some(watching) if !watching.listing => {
                watching.listing = true;
                watching.on_session = false;
                self.scroll = 0;
                true
            }
            _ => false,
        }
    }

    /// Move to the next delegate, or to the previous one, in the order they were spawned.
    ///
    /// Each stops at its end rather than wrapping: somebody stepping through wants to arrive at
    /// the last one and know that it is the last.
    ///
    /// The session is one of the rows in the list, above the first delegate, so moving up from
    /// that one reaches it. It is not one of the steps in a delegate's own view: what `n` and `p`
    /// are for there is comparing two runs, and a key that stepped out of the mode partway
    /// through would be a different key wearing the same name.
    pub fn watch_next(&mut self) {
        let last = self.watchable().len().saturating_sub(1);
        let Some(watching) = &mut self.watching else {
            return;
        };
        if watching.on_session {
            watching.on_session = false;
            watching.at = 0;
            self.scroll = 0;
        } else if watching.at < last {
            watching.at += 1;
            self.scroll = 0;
        }
    }

    pub fn watch_previous(&mut self) {
        let Some(watching) = &mut self.watching else {
            return;
        };
        if watching.on_session {
            return;
        }
        if watching.at > 0 {
            watching.at -= 1;
            self.scroll = 0;
        } else if watching.listing {
            watching.on_session = true;
            self.scroll = 0;
        }
    }

    /// The entries the screen is drawn from: one delegate's own, or the turn's.
    ///
    /// The single place the two views part company, so everything that lays out a transcript does
    /// it the same way for both and neither can drift from the other.
    pub fn viewed(&self) -> &[Entry] {
        match self.watching.filter(|watching| !watching.listing) {
            Some(watching) => match self.delegates().get(watching.at).copied() {
                Some(delegate) => &delegate.lines,
                None => &self.transcript,
            },
            None => &self.transcript,
        }
    }

    /// Show a tool call that has begun.
    pub fn start_activity(&mut self, activity: Activity) {
        self.running = Some(activity.clone());
        let entry = Entry::tool(activity);
        match self.attributed_to.and_then(|id| self.at(id)) {
            Some(at) => self.transcript[at]
                .delegate
                .as_mut()
                .expect("a delegate entry holds its delegate")
                .keep(entry),
            None => self.transcript.push(entry),
        }
    }

    /// Record where the last call's result went.
    pub fn landed(&mut self, landing: Landing) {
        if let Some(entry) = self.working_lines().last_mut()
            && entry.speaker == Speaker::Tool
        {
            entry.landing = Some(landing);
        }
    }

    /// Show the person quarantined content the planner was not shown.
    ///
    /// Attached to the call that produced it, so it reads as part of that line rather than as
    /// something the session said. Where there is no such line, which should not happen, it goes
    /// on its own rather than being dropped: content released for a screen and then not drawn is
    /// the worst of both.
    pub fn show(&mut self, shown: Shown) {
        self.scroll = 0;
        match self.working_lines().last_mut() {
            Some(entry) if entry.speaker == Speaker::Tool && entry.shown.is_none() => {
                entry.shown = Some(shown);
            }
            _ => {
                let mut entry = Entry::system("");
                entry.shown = Some(shown);
                self.working_lines().push(entry);
            }
        }
    }

    /// Replace the call in flight with how it turned out.
    ///
    /// Matched by position, not by name: only one call runs at a time, so the running entry at
    /// the end of the transcript is necessarily the one that just finished. A finish with no
    /// start before it is appended rather than dropped, since losing the record of a call that
    /// happened is worse than an unpaired line.
    pub fn finish_activity(&mut self, activity: Activity) {
        self.running = None;
        match self.working_lines().last_mut() {
            Some(entry) if entry.speaker == Speaker::Tool && Self::still_running(entry) => {
                *entry = Entry::tool(activity);
            }
            // Straight into the delegate's own count where it has one, since a call that
            // finished without this side seeing it start is still a call it made.
            _ => match self.attributed_to.and_then(|id| self.at(id)) {
                Some(at) => self.transcript[at]
                    .delegate
                    .as_mut()
                    .expect("a delegate entry holds its delegate")
                    .keep(Entry::tool(activity)),
                None => self.transcript.push(Entry::tool(activity)),
            },
        }
    }

    fn still_running(entry: &Entry) -> bool {
        entry.activity.as_ref().is_some_and(Activity::is_running)
    }

    /// Accept a typed character.
    ///
    /// Allowed while a turn runs as well as between turns. What it cannot do then is send: a
    /// second turn must not begin while the first is in flight, and [`Session::submit`] still
    /// refuses. Dropping the keys instead, which is what this used to do, meant a user typing
    /// during a slow turn watched their words go nowhere with nothing to say why.
    pub fn type_char(&mut self, c: char) {
        // Before everything, including the two markers below. In NORMAL mode a letter is an
        // instruction, and that is the whole of what the mode means: `!` and `?` there are vi's own
        // keys rather than the ways shell mode and the key list are opened, and reading either as its
        // marker would answer a press that was asking for something else. Both are a press of `i`
        // away for somebody who wanted them.
        if self.vi_normal() {
            self.obey(c);
            return;
        }
        // `?` on an empty line puts the list of keys up rather than typing a character, and a second
        // press takes it down again. Only on an empty line, since a `?` in a sentence is the
        // punctuation somebody is asking a question with, and not in shell mode, where it is a glob
        // for the shell to expand.
        //
        // First, so the press that closes the list is not also the press that clears it below.
        if c == '?' && self.input.is_empty() && !self.shell {
            self.shortcuts = !self.shortcuts;
            return;
        }
        // Any other key takes it down. The question has been asked and moved on from, and somebody
        // typing again has finished reading.
        self.shortcuts = false;

        // `!` on an empty line is the mode rather than a character, which is what makes the rest of
        // the line the command verbatim. Only on an empty line: a `!` inside a sentence is
        // punctuation, and inside a command it is history expansion for the shell to deal with.
        //
        // Idle only. The mode is an armed state that changes what Enter does, and mid-turn the user
        // cannot act on it: it would still be armed when the turn ended, over whatever the box held
        // by then. A cancelled turn puts the prompt back, so `!` during one used to leave a sentence
        // sitting behind a marker, and "rm the old builds" is a reasonable thing to have typed.
        if c == '!' && self.input.is_empty() && !self.shell && self.status == Status::Idle {
            self.shell = true;
            return;
        }
        // Editing a recalled prompt makes it the working line rather than a view of history,
        // so the position indicator goes away as soon as a key is pressed.
        self.history.leave();
        self.input.insert(self.caret, c);
        self.caret += c.len_utf8();
        // Back to the top of whatever is now offered. A cursor left where it was would sit on a
        // different command after one more letter, so the highlighted row would drift as the list
        // narrowed under it.
        self.completion = 0;
    }

    /// Start a new line in the prompt without sending it.
    ///
    /// Not [`Session::type_char`] with a newline, because that would read a leading `!` as shell
    /// mode and would let the character be typed by any path that thinks it is typing text. A
    /// newline in the prompt is one deliberate keystroke.
    pub fn type_newline(&mut self) {
        self.history.leave();
        self.input.insert(self.caret, '\n');
        self.caret += 1;
        // A command is one line by definition, and a reference ends at whitespace, so a newline
        // closes whatever was being offered rather than narrowing it.
        self.completion = 0;
    }

    /// Which style of editing the box does.
    pub fn editing(&self) -> crate::vim::Editing {
        self.editing
    }

    /// Settle the style of editing for this session, given what a settings file said.
    ///
    /// A choice the person made outranks the file, the same rule the model follows: a recorded choice
    /// outlives the session that made it, and a file read afterwards would undo what somebody had just
    /// asked for. With no choice recorded the file answers, and with neither it is the box everybody
    /// has.
    ///
    /// A word the file spelled that names no style leaves the ordinary box. Nothing is said about it
    /// here: a settings file is reported by `doctor`, and a session that refused to start over a
    /// mistyped editing preference would be worse than one that ignores it.
    ///
    /// The recorded choice is read only for a session that persists, which is the rule every write
    /// here follows: a test must not be handed the developer's own preference, or what the box does
    /// under it would depend on the machine it ran on.
    pub fn adopt_editing(&mut self, configured: Option<&str>) {
        let chosen = self.persist.then(crate::store::load_editing).flatten();
        self.editing = chosen
            .or_else(|| configured.and_then(crate::vim::Editing::named))
            .unwrap_or_default();
    }

    /// Record the style of editing the person chose, keeping it for later sessions.
    ///
    /// Written through to disk only for a session that persists, the same rule the model and the
    /// effort level follow and for the same reason: a test must not rewrite the developer's own
    /// choice.
    ///
    /// The mode comes back to INSERT whichever style was chosen, because the choice is made away from
    /// the box: coming back to one that takes the next letter as an instruction is not what somebody
    /// who has just turned vi editing on expects. It is also the only sound answer for the other
    /// direction, NORMAL not being a state the ordinary box has.
    pub fn choose_editing(&mut self, editing: crate::vim::Editing) {
        if self.persist {
            crate::store::save_editing(editing);
        }
        self.editing = editing;
        self.mode = crate::vim::Mode::Insert;
    }

    /// Which vi mode the box is in, or `None` where vi is not the style.
    ///
    /// `None` rather than INSERT for the ordinary box, so nothing can draw a mode at somebody who
    /// never asked for one.
    pub fn vi_mode(&self) -> Option<crate::vim::Mode> {
        match self.editing {
            crate::vim::Editing::Vi => Some(self.mode),
            crate::vim::Editing::Ordinary => None,
        }
    }

    /// Whether a letter typed now is an instruction rather than a letter.
    pub fn vi_normal(&self) -> bool {
        self.vi_mode() == Some(crate::vim::Mode::Normal)
    }

    /// Take the letters as instructions, which is what Escape asks for.
    ///
    /// `false` where vi is not the style and nothing happened. Callers read the style themselves,
    /// since a key press is chosen between several meanings and a call that mutates while that choice
    /// is being made is one whose order of evaluation is load-bearing. The refusal is here as well
    /// because it is the property worth holding whatever a caller does: the ordinary box has no
    /// NORMAL mode to be put into, so nothing reachable can leave it in one.
    pub fn enter_vi_normal(&mut self) -> bool {
        if self.editing != crate::vim::Editing::Vi {
            return false;
        }
        self.mode = crate::vim::Mode::Normal;
        // Where vi leaves it. The caret in NORMAL mode sits on a character rather than between two,
        // so the position one past the end of the line is not one it can hold, and Escape at the end
        // of a line somebody has just typed lands on the last character they typed.
        if self.caret == self.input.len() {
            self.move_left();
        }
        true
    }

    /// Carry out the instruction a letter is in NORMAL mode.
    ///
    /// A letter vi does not use does nothing at all, which is the mode's whole bargain: the box is
    /// not typing, so an instruction it does not recognise is not text to fall back on.
    fn obey(&mut self, c: char) {
        // A key that was waiting for one more takes this press and nothing else looks at it. Cleared
        // first, so a pair that means nothing ends the wait rather than holding it open: one stray
        // press would otherwise swallow every letter after it until something happened to match.
        if let Some(pending) = self.half_typed.take() {
            self.carry_out(pending.then(c));
            return;
        }
        // The keys that repeat a jump, which mean nothing on their own: they say "that again", and
        // what "that" was is the only thing the session remembers about a motion.
        if let (';' | ',', Some(find)) = (c, self.last_find) {
            let repeated = if c == ';' { find } else { find.reversed() };
            self.jump_to_char(repeated);
            return;
        }
        self.carry_out(crate::vim::command(c));
    }

    /// Act on an instruction that has everything it needs.
    fn carry_out(&mut self, command: crate::vim::Command) {
        use crate::vim::Command;

        match command {
            Command::Insert(opening) => self.open_insert(opening),
            Command::Move(motion) => self.move_by(motion),
            Command::Change(operator, extent) => self.change(operator, extent),
            Command::Wait(pending) => self.half_typed = Some(pending),
            Command::Undo => self.undo_last_change(),
            Command::Again => {
                if let Some((operator, extent)) = self.last_change {
                    self.change(operator, extent);
                }
            }
            Command::Paste { before } => self.put_the_register_back(before),
            Command::Join => self.join_the_line_below(),
            Command::Nothing => {}
        }
    }

    /// Do something to the stretch of the line an extent names.
    ///
    /// The one place a vi instruction changes the line, so the register, the undo step and the record
    /// of what `.` repeats are all kept here. Three copies of that bookkeeping, one per operator, is
    /// how one of them would come to be missing.
    fn change(&mut self, operator: crate::vim::Operator, extent: crate::vim::Extent) {
        use crate::vim::Operator;

        let Some((from, to)) = self.stretch(self.as_vi_reads_it(operator, extent)) else {
            return;
        };

        if !operator.reads_only() {
            self.before_last_change = Some((self.input.clone(), self.caret));
            self.last_change = Some((operator, extent));
            self.history.leave();
            self.completion = 0;
        }

        let whole_lines = extent == crate::vim::Extent::Line;
        self.register = Some(Yanked {
            text: self.input[from..to].to_string(),
            lines: whole_lines,
        });

        match operator {
            // Nothing moves, so the caret has no reason to be anywhere but where the yank began, which
            // is where vi leaves it.
            Operator::Yank => self.caret = from,
            Operator::Change => {
                self.input.replace_range(from..to, "");
                self.caret = from;
                self.mode = crate::vim::Mode::Insert;
            }
            Operator::Delete => {
                // A whole line takes the newline that ends it, so the gap closes rather than leaving a
                // blank line where the line was. The one before it on the last line, or the line above
                // gains a trailing newline it never had. `cc` is the other way and keeps it, which is
                // what leaves the person typing on the line they asked to replace.
                let (from, to) = if whole_lines {
                    match (to < self.input.len(), from > 0) {
                        (true, _) => (from, to + 1),
                        (false, true) => (from - 1, to),
                        (false, false) => (from, to),
                    }
                } else {
                    (from, to)
                };
                self.input.replace_range(from..to, "");
                self.caret = from;
                // The caret has to land on a character, and taking the end of a line leaves it past the
                // last one.
                self.step_back_off_the_end();
            }
            Operator::Indent | Operator::Dedent => {
                self.shift_the_line(operator == Operator::Indent)
            }
        }
    }

    /// The extent an operator actually acts on, which is not always the one the keys spelled.
    ///
    /// `cw` on a character that is not a blank means `ce`: it changes the word and leaves the space
    /// after it, where `dw` takes that space. This is vi's own special case rather than something
    /// derivable, and it exists because the alternative is useless: somebody replacing a word almost
    /// never wants it run into the next one, and typing the space back every time is what `cw` would
    /// otherwise cost. On a blank there is no word to change, and it means `dw` again.
    ///
    /// Measured against vim itself rather than reasoned about, since a special case is a fact about
    /// what people's hands expect and not something the rest of this can predict.
    fn as_vi_reads_it(
        &self,
        operator: crate::vim::Operator,
        extent: crate::vim::Extent,
    ) -> crate::vim::Extent {
        use crate::vim::{Extent, Motion, Operator};

        let on_a_blank = self.input[self.caret..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace);
        match (operator, extent) {
            (Operator::Change, Extent::To(Motion::WordRight)) if !on_a_blank => {
                Extent::To(Motion::WordEnd)
            }
            _ => extent,
        }
    }

    /// The stretch of the line an extent names, as byte offsets, or `None` where it names nothing.
    ///
    /// Mostly worked out by moving the caret and reading where it lands, then putting it back, which is
    /// what makes those extents obey the marker rules for free: the ends are positions the caret could
    /// rest at, so no stretch can begin or end inside a marker.
    ///
    /// A text object is the exception, since it is found by reading the line rather than by walking, and
    /// a marker is spelled with brackets and a digit: `di[` on one named the brackets it is written with
    /// and left `[]` standing for nothing. So every stretch is checked against the markers before it is
    /// returned, once here rather than in each kind of object.
    fn stretch(&mut self, extent: crate::vim::Extent) -> Option<(usize, usize)> {
        let (from, to) = self.stretch_unchecked(extent)?;
        // Widened to whole markers rather than refused, so an object that reached into one takes it with
        // what it was already taking. Refusing would leave `daw` over a marker doing nothing at all,
        // where taking the marker is plainly what was asked for.
        let from = self
            .marker_spans()
            .find(|(start, end)| *start < from && from < *end)
            .map_or(from, |(start, _)| start);
        let to = self
            .marker_spans()
            .find(|(start, end)| *start < to && to < *end)
            .map_or(to, |(_, end)| end);
        Some((from, to)).filter(|(from, to)| from < to)
    }

    /// The stretch an extent names, before the markers are taken into account.
    fn stretch_unchecked(&mut self, extent: crate::vim::Extent) -> Option<(usize, usize)> {
        use crate::vim::Extent;

        let was = self.caret;
        let span = match extent {
            // The line's own characters, and not the newline that ends it. What happens to that
            // newline is the operator's business rather than the extent's: `dd` takes it so the gap
            // closes, `cc` leaves it so the person is typing on the line they asked to replace, and
            // `yy` records the content and puts a newline back when it lands.
            Extent::Line => Some(self.caret_line()),
            Extent::ToLineEnd => Some((self.caret, self.caret_line().1)),
            Extent::Character => {
                // Whole where it is a marker, which is what the caret is on rather than the bracket it
                // begins with.
                match self.marker_at_caret() {
                    Some(span) => Some(span),
                    None => self.input[self.caret..]
                        .chars()
                        .next()
                        .map(|c| (self.caret, self.caret + c.len_utf8())),
                }
            }
            Extent::To(motion) => {
                self.move_by_for(motion, true);
                let landed = self.caret;
                match landed.cmp(&was) {
                    std::cmp::Ordering::Equal => None,
                    // Forwards, and the character landed on is taken or not depending on the motion:
                    // `de` takes the word's last letter and `dw` stops before the next word's first.
                    std::cmp::Ordering::Greater => {
                        let end = if motion.takes_what_it_lands_on() {
                            self.after_the_caret()
                        } else {
                            landed
                        };
                        Some((was, end))
                    }
                    // Backwards, where the character the caret started on is the one not asked for:
                    // `db` takes back to the start of the word and leaves what the caret was on.
                    std::cmp::Ordering::Less => Some((landed, was)),
                }
            }
            Extent::Object(object) => self.object_span(object),
        };
        self.caret = was;
        span.filter(|(from, to)| from < to)
    }

    /// The stretch a text object names, on the line the caret is on.
    ///
    /// The line rather than the whole input, for the reason a jump to a character stays on its own
    /// line: these keys are for the thing in front of you, and one that reached across a newline would
    /// take part of a paragraph nobody was looking at.
    fn object_span(&self, object: crate::vim::Object) -> Option<(usize, usize)> {
        use crate::vim::Kind;

        match object.kind {
            Kind::Word => self.run_span(object.around, |c| {
                if c.is_whitespace() {
                    Class::Blank
                } else if c.is_alphanumeric() || c == '_' {
                    Class::Word
                } else {
                    Class::Punctuation
                }
            }),
            // Anything but a blank is one thing, which is what makes a path or a flag a single object.
            Kind::Bigword => self.run_span(object.around, |c| {
                if c.is_whitespace() {
                    Class::Blank
                } else {
                    Class::Word
                }
            }),
            Kind::Pair(opens, closes) => self.pair_span(opens, closes, object.around),
        }
    }

    /// The run of same-kind characters the caret is in, and the blanks after it where asked for.
    ///
    /// A run of blanks is itself a run, which is what makes `diw` on a space take the spaces: the caret
    /// is in something, and the object is whatever it is in.
    fn run_span(&self, around: bool, class: impl Fn(char) -> Class) -> Option<(usize, usize)> {
        let (line_start, line_end) = self.caret_line();
        let line = &self.input[line_start..line_end];
        let at = self.caret.min(line_end) - line_start;
        let here = class(line[at..].chars().next()?);

        let from = line[..at]
            .char_indices()
            .rev()
            .take_while(|(_, c)| class(*c) == here)
            .last()
            .map_or(at, |(index, _)| index);
        let mut to = at
            + line[at..]
                .char_indices()
                .take_while(|(_, c)| class(*c) == here)
                .map(|(index, c)| index + c.len_utf8())
                .last()
                .unwrap_or(0);

        // `aw` takes the blanks after the word as well, which is what makes it the whole word rather
        // than the word alone: deleting one and leaving two spaces behind is not what was asked for.
        if around {
            let after = to
                + line[to..]
                    .char_indices()
                    .take_while(|(_, c)| class(*c) == Class::Blank && here != Class::Blank)
                    .map(|(index, c)| index + c.len_utf8())
                    .last()
                    .unwrap_or(0);
            // Nothing after it, so the blanks before it are what `aw` takes instead: on the last word
            // of a line, taking nothing extra would make `daw` the same as `diw`.
            if after == to && here != Class::Blank {
                let before = line[..from]
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| class(*c) == Class::Blank)
                    .last()
                    .map_or(from, |(index, _)| index);
                return Some((line_start + before, line_start + to));
            }
            to = after;
        }
        Some((line_start + from, line_start + to))
    }

    /// The stretch a pair of delimiters names, with or without the delimiters themselves.
    ///
    /// The pair the caret is inside, or else the next one along the line. That second half is what makes
    /// `ci(` work with the caret on the name in front of the bracket, which is where it usually is.
    fn pair_span(&self, opens: char, closes: char, around: bool) -> Option<(usize, usize)> {
        let (line_start, line_end) = self.caret_line();
        let line = &self.input[line_start..line_end];
        let at = self.caret.min(line_end) - line_start;

        let (from, to) = if opens == closes {
            // A quote is its own closing mark, so there is no nesting to count and the pairs are the
            // marks taken two at a time from the start of the line.
            let marks: Vec<usize> = line
                .char_indices()
                .filter(|(_, c)| *c == opens)
                .map(|(index, _)| index)
                .collect();
            marks
                .chunks_exact(2)
                .map(|pair| (pair[0], pair[1]))
                .find(|(open, close)| at <= *close || at <= *open)?
        } else {
            // Counted rather than matched by position, so a bracket inside a bracket is the pair that
            // encloses the caret rather than whichever one came first.
            let enclosing = self.enclosing_pair(line, at, opens, closes);
            match enclosing {
                Some(pair) => pair,
                None => self.next_pair(line, at, opens, closes)?,
            }
        };

        if !around {
            return Some((line_start + from + opens.len_utf8(), line_start + to));
        }

        // `a"` takes the blanks in front of the quotes and `a(` does not, which is vim's own
        // inconsistency and measurably what it does. A quote has no shape of its own on the line, so
        // what it delimits reads as a word and the blank beside it belongs to it; a bracket usually
        // follows the name it belongs to, and taking the space would take part of that name's spacing.
        let ends = to + closes.len_utf8();
        let from = if opens == closes {
            line[..from]
                .char_indices()
                .rev()
                .take_while(|(_, c)| *c == ' ')
                .last()
                .map_or(from, |(index, _)| index)
        } else {
            from
        };
        Some((line_start + from, line_start + ends))
    }

    /// The innermost pair of delimiters the caret lies within, counting nesting.
    fn enclosing_pair(
        &self,
        line: &str,
        at: usize,
        opens: char,
        closes: char,
    ) -> Option<(usize, usize)> {
        let mut open_at = Vec::new();
        for (index, c) in line.char_indices() {
            if c == opens {
                open_at.push(index);
            } else if c == closes
                && let Some(from) = open_at.pop()
                && from <= at
                && at <= index
            {
                return Some((from, index));
            }
        }
        None
    }

    /// The first complete pair of delimiters beginning at or after the caret.
    fn next_pair(
        &self,
        line: &str,
        at: usize,
        opens: char,
        closes: char,
    ) -> Option<(usize, usize)> {
        let from = line[at..]
            .char_indices()
            .find(|(_, c)| *c == opens)
            .map(|(index, _)| at + index)?;
        let mut depth = 0usize;
        for (index, c) in line[from..].char_indices() {
            if c == opens {
                depth += 1;
            } else if c == closes {
                depth -= 1;
                if depth == 0 {
                    return Some((from, from + index));
                }
            }
        }
        None
    }

    /// Move the line the caret is on towards or away from the margin.
    ///
    /// A fixed step of spaces rather than a tab, because the box draws what it holds and a tab's width
    /// is the terminal's opinion: a line indented with one would sit somewhere different here than in
    /// the file it was copied from.
    fn shift_the_line(&mut self, further: bool) {
        /// How far one press moves a line.
        const STEP: usize = 2;

        let (start, _) = self.caret_line();
        if further {
            self.input.insert_str(start, &" ".repeat(STEP));
            self.caret += STEP;
            return;
        }
        let blanks = self.input[start..]
            .chars()
            .take(STEP)
            .take_while(|c| *c == ' ')
            .count();
        self.input.replace_range(start..start + blanks, "");
        self.caret = self.caret.saturating_sub(blanks).max(start);
    }

    /// Put the register back into the line, beside the caret or as a line of its own.
    ///
    /// Nothing where nothing has been yanked, rather than a guess at what to insert.
    fn put_the_register_back(&mut self, before: bool) {
        let Some(yanked) = self.register.clone() else {
            return;
        };
        self.before_last_change = Some((self.input.clone(), self.caret));
        self.history.leave();
        self.completion = 0;

        if yanked.lines {
            // A yanked line goes back as a line rather than into the middle of the one the caret is
            // on, which is the whole reason the register remembers which it was.
            // The register holds the line's characters without the newline that ended it, so the
            // newline goes on whichever side puts the text on a line of its own.
            let (start, end) = self.caret_line();
            let (at, text) = if before {
                (start, format!("{}\n", yanked.text))
            } else {
                (end, format!("\n{}", yanked.text))
            };
            self.input.insert_str(at, &text);
            self.caret = if before { start } else { at + 1 };
            return;
        }

        // `p` puts it after the character the caret is on, which is where vi puts it: the caret is on a
        // character rather than between two, so there is no position that means "here" for both keys.
        let at = if before {
            self.caret
        } else {
            self.after_the_caret()
        };
        self.input.insert_str(at, &yanked.text);
        self.caret = at + yanked.text.len();
        self.step_back_off_the_end();
    }

    /// The position just past the character the caret is on, which is where `p` inserts.
    fn after_the_caret(&self) -> usize {
        if let Some((_, end)) = self.marker_at_caret() {
            return end;
        }
        match self.input[self.caret..].chars().next() {
            Some(c) => self.caret + c.len_utf8(),
            None => self.caret,
        }
    }

    /// Make this line and the one below into one, which is what `J` asks for.
    ///
    /// The newline becomes a single space, which is what vi does: two sentences run together with no
    /// gap is not what somebody joining lines wants, and the blanks the next line was indented with are
    /// part of the shape it no longer has.
    fn join_the_line_below(&mut self) {
        let (_, end) = self.caret_line();
        if end >= self.input.len() {
            return;
        }
        self.before_last_change = Some((self.input.clone(), self.caret));
        self.history.leave();
        self.completion = 0;

        let below = end + 1;
        let text = self.input[below..].to_string();
        let blanks = text.len() - text.trim_start_matches([' ', '\t']).len();
        self.input.replace_range(end..below + blanks, " ");
        self.caret = end;
    }

    /// Put the line back as it stood before the last change.
    ///
    /// Nothing where no change has been made. One step rather than a stack, on the same footing as the
    /// stash: the press that undoes and the keystroke that will be regretted are one apart.
    fn undo_last_change(&mut self) {
        let Some((line, caret)) = self.before_last_change.take() else {
            return;
        };
        self.input = line;
        self.caret = caret;
        self.history.leave();
        self.completion = 0;
        self.step_back_off_the_end();
    }

    /// The key a press in NORMAL mode stands for, where vi spells an existing binding with a letter.
    ///
    /// `j` and `k` are Down and Up, and `/` is the chord that searches the prompts already sent. Named
    /// here and answered by the key handler rather than acted on from this side, because what those
    /// keys reach is not the line: at the ends of the input they walk the prompt history and then
    /// scroll the transcript, and a letter that moved the caret from here would never reach either.
    ///
    /// Nothing while a key is waiting for one more, where every character is that one: `f/` jumps to
    /// a slash rather than opening a search.
    pub fn vi_spells(&self, c: char) -> Option<Spelled> {
        if !self.vi_normal() || self.half_typed.is_some() {
            return None;
        }
        match c {
            'j' => Some(Spelled::Down),
            'k' => Some(Spelled::Up),
            // `/` searches in vi, and the prompts already sent are the only thing here to search.
            '/' => Some(Spelled::SearchPrompts),

            _ => None,
        }
    }

    /// Move the caret where a motion says.
    ///
    /// Every one of these goes through the caret methods the arrows already use, so a marker is
    /// crossed whole and there is no position inside one for a motion to leave the caret at. A motion
    /// that did byte arithmetic on the line would have to know about markers itself, and the one that
    /// forgot would be the one that put the caret in the middle of a picture.
    fn move_by(&mut self, motion: crate::vim::Motion) {
        self.move_by_for(motion, false);
    }

    /// The same motion, told whether it is moving the caret or measuring a stretch for an operator.
    ///
    /// The two differ in one place: the position after the last character of the line. The caret cannot
    /// rest there, but it is where a stretch ending at that character ends, so a motion clamped for
    /// both would leave `dl` on a final character measuring nothing at all.
    fn move_by_for(&mut self, motion: crate::vim::Motion, measuring: bool) {
        use crate::vim::Motion;

        match motion {
            Motion::Left => self.move_left(),
            // Stopping on the last character rather than the column after it, which is where the
            // arrows leave the caret in INSERT mode and is not a position NORMAL mode has.
            //
            // Except when a stretch is being measured, where the position after the last character is
            // the end of that stretch rather than somewhere the caret will rest: clamped, `dl` on the
            // final character measures nothing and the key that means `x` would do nothing there.
            Motion::Right => {
                self.move_right();
                if !measuring {
                    self.step_back_off_the_end();
                }
            }
            Motion::WordRight => self.move_word_start_right(),
            Motion::WordEnd => self.move_word_end_right(),
            Motion::WordLeft => self.move_word_left(),
            Motion::LineStart => self.move_to_line_start(),
            // The last character rather than the position after it, since that is not one the caret
            // can hold in NORMAL mode.
            Motion::LineEnd => {
                self.move_to_line_end();
                self.step_back_off_the_end();
            }
            Motion::FirstNonBlank => self.move_to_first_non_blank(),
            Motion::InputStart => self.caret = 0,
            Motion::InputEnd => {
                self.caret = self.input.len();
                self.move_to_line_start();
            }
            Motion::ToChar(find) => {
                self.last_find = Some(find);
                self.jump_to_char(find);
            }
        }
    }

    /// Put the caret on a character rather than past the end of the line.
    ///
    /// NORMAL mode's caret sits on the character the next instruction acts on, and the column after
    /// the line holds none. Nothing to do on an empty line, which has no character to sit on either.
    fn step_back_off_the_end(&mut self) {
        let (start, _) = self.caret_line();
        if self.caret > start && self.at_line_end() {
            self.move_left();
        }
    }

    /// Whether the caret is at the end of the line it is on.
    fn at_line_end(&self) -> bool {
        self.caret == self.caret_line().1
    }

    /// Move to the first character of the line that is not a blank, which is what `^` asks for.
    fn move_to_first_non_blank(&mut self) {
        self.move_to_line_start();
        while !self.at_line_end()
            && self.input[self.caret..]
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace())
        {
            self.move_right();
        }
    }

    /// Move to the start of the next word, which is what `w` asks for.
    ///
    /// Different from the word motion the arrows use under Ctrl: that one lands after the word it
    /// crossed, and this one lands on the first character of the next. Both are wanted, and vi's is
    /// the one an instruction typed as `w` has to mean.
    fn move_word_start_right(&mut self) {
        let was = self.caret;
        // Out of the word the caret is in, then over the blanks after it. A caret already on a blank
        // skips the first loop and lands on the next word, which is the same answer.
        while !self.at_input_end()
            && self.input[self.caret..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace())
        {
            self.move_right();
        }
        while !self.at_input_end()
            && self.input[self.caret..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            self.move_right();
        }
        // At the end of the input there is no next word, so the caret stays where it was rather than
        // coming to rest past the last character.
        if self.at_input_end() {
            self.caret = was;
            self.move_to_line_end();
            self.step_back_off_the_end();
        }
    }

    /// Move to the end of this word, or of the next one where the caret is already at an end.
    ///
    /// Which is what `e` asks for, and why it is not `w` stepped back: on the last character of a
    /// word it has to reach the end of the following one.
    fn move_word_end_right(&mut self) {
        let was = self.caret;
        self.move_right();
        while !self.at_input_end()
            && self.input[self.caret..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            self.move_right();
        }
        while !self.at_input_end()
            && self.input[self.caret..]
                .chars()
                .nth(1)
                .is_some_and(|c| !c.is_whitespace())
        {
            self.move_right();
        }
        if self.at_input_end() {
            self.caret = was.max(self.last_caret_position());
        }
    }

    /// Whether the caret is at the end of the whole input.
    fn at_input_end(&self) -> bool {
        self.caret >= self.input.len()
    }

    /// The last position the caret can hold in NORMAL mode: on the final character, not past it.
    fn last_caret_position(&self) -> usize {
        match self.input.chars().next_back() {
            Some(c) => self.input.len() - c.len_utf8(),
            None => 0,
        }
    }

    /// Jump to a character on the line the caret is on, if it holds one.
    ///
    /// The line rather than the whole input, which is what vi does: these keys are for reaching a
    /// bracket or a comma in front of you, and one that crossed a newline would land somewhere off
    /// the row being read. A character that is not there leaves the caret alone.
    /// A marker is one thing to the caret, so a target inside one is not somewhere to land: the caret
    /// would sit between two halves of a picture, in a place the person cannot see, and the next
    /// instruction would act there. A marker is spelled with brackets and a digit, so `f]` and `f1`
    /// both name characters that are inside one.
    ///
    /// Walked a position at a time through the caret methods rather than found by searching the bytes,
    /// which is what keeps this true without a second copy of the marker rules living here.
    fn jump_to_char(&mut self, find: crate::vim::Find) {
        let was = self.caret;
        let (start, end) = self.caret_line();
        let step = |session: &mut Self| {
            if find.forwards {
                session.move_right();
            } else {
                session.move_left();
            }
        };
        let arrived = |session: &Self| {
            if find.forwards {
                session.caret >= end
            } else {
                session.caret <= start
            }
        };

        // Off the character the caret is on before looking, so `f` twice reaches the second occurrence
        // rather than staying on the first.
        step(self);
        while !arrived(self) {
            if self.input[self.caret..].starts_with(find.target) {
                // `t` and `T` stop one character short of the target, on the side the jump came from.
                if find.short {
                    if find.forwards {
                        self.move_left();
                    } else {
                        self.move_right();
                    }
                }
                return;
            }
            step(self);
        }

        // The character is not on this line, and a jump to nothing leaves the caret where it was
        // rather than at whichever end the walk gave up at.
        self.caret = was;
    }

    /// Take the letters as letters again, with the caret where the key asked for it.
    fn open_insert(&mut self, opening: crate::vim::Opening) {
        use crate::vim::Opening;

        self.mode = crate::vim::Mode::Insert;
        match opening {
            Opening::Here => {}
            Opening::LineStart => self.move_to_line_start(),
            // Onto the position after the character the caret is on, which in INSERT mode is where
            // the next thing typed lands. On an empty line there is nowhere to move and `a` is `i`.
            Opening::After => self.move_right(),
            Opening::LineEnd => self.move_to_line_end(),
            Opening::LineBelow => {
                self.move_to_line_end();
                self.type_newline();
            }
            Opening::LineAbove => {
                self.move_to_line_start();
                self.type_newline();
                // The newline went in before the caret, so the caret is now on the line that was
                // there and the empty one is above it. Stepping back puts it on the empty one.
                self.move_left();
            }
        }
    }

    /// The line being typed.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Where the next keystroke will land, as a byte offset into the line.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Replace the line, leaving the caret where the user would carry on typing.
    ///
    /// Everything that puts a whole line in the box goes through here, so no path can leave the
    /// caret pointing into a line that is no longer there.
    ///
    /// The list of keys goes with it, for the same reason and in the same one place. It is not part
    /// of the line and cannot be rewritten with it, but it stands over the box, and a list left up
    /// over a line that arrived under it belongs to a press two prompts ago. Several callers took it
    /// down themselves and the ones that did not were the ones a person reached mid-turn: recalling
    /// an earlier prompt, and a stopped turn handing its prompt back.
    fn set_input(&mut self, line: impl Into<String>) {
        self.input = line.into();
        self.caret = self.input.len();
        self.shortcuts = false;
    }

    /// Whether the line has more than one line in it, which is what gives Up and Down something
    /// to move between.
    pub fn is_multiline(&self) -> bool {
        self.input.contains('\n')
    }

    /// The line the caret is on, as byte offsets into the input.
    fn caret_line(&self) -> (usize, usize) {
        let start = self.input[..self.caret]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        let end = self.input[self.caret..]
            .find('\n')
            .map_or(self.input.len(), |newline| self.caret + newline);
        (start, end)
    }

    /// Move the caret one character towards the start, counting a marker as one.
    ///
    /// A marker stands for one thing, and a caret resting in the middle of one would be a caret
    /// between two halves of a picture. So it is stepped over whole, in either direction, and the
    /// places the caret can rest are the same places the user can see.
    pub fn move_left(&mut self) {
        if let Some((start, _)) = self.marker_before_caret() {
            self.caret = start;
            return;
        }
        if let Some(c) = self.input[..self.caret].chars().next_back() {
            self.caret -= c.len_utf8();
        }
    }

    /// Move the caret one character towards the end, counting a marker as one.
    pub fn move_right(&mut self) {
        if let Some((_, end)) = self.marker_at_caret() {
            self.caret = end;
            return;
        }
        if let Some(c) = self.input[self.caret..].chars().next() {
            self.caret += c.len_utf8();
        }
    }

    /// Move the caret to the start of the word before it.
    ///
    /// Words are runs of anything but whitespace, which is what makes a path or a flag one word:
    /// stopping inside `--file` or `src/main.rs` would be several presses to cross something the
    /// user thinks of as one thing.
    pub fn move_word_left(&mut self) {
        while self.input[..self.caret]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
        {
            self.move_left();
        }
        while self.input[..self.caret]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace())
        {
            self.move_left();
        }
    }

    /// Move the caret to the end of the word after it.
    pub fn move_word_right(&mut self) {
        while self.input[self.caret..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            self.move_right();
        }
        while self.input[self.caret..]
            .chars()
            .next()
            .is_some_and(|c| !c.is_whitespace())
        {
            self.move_right();
        }
    }

    /// Move the caret to the start of the line it is on.
    pub fn move_to_line_start(&mut self) {
        self.caret = self.caret_line().0;
    }

    /// Move the caret to the end of the line it is on.
    pub fn move_to_line_end(&mut self) {
        self.caret = self.caret_line().1;
    }

    /// Move the caret to the start of its line, or to the start of the line above when it is
    /// already there.
    ///
    /// `false` when there was nowhere left to go, so the caller can leave the key to the transcript
    /// at the top of the prompt rather than having it do nothing.
    ///
    /// Two presses to cross a line boundary, which is what makes the first press cheap: someone who
    /// wanted the start of this line gets it without also losing their place in the paragraph.
    pub fn page_up(&mut self) -> bool {
        let (start, _) = self.caret_line();
        if self.caret > start {
            self.caret = start;
            return true;
        }
        if start == 0 {
            return false;
        }
        self.caret = self.input[..start - 1]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        true
    }

    /// Move the caret to the end of its line, or to the end of the line below when it is already
    /// there.
    pub fn page_down(&mut self) -> bool {
        let (_, end) = self.caret_line();
        if self.caret < end {
            self.caret = end;
            return true;
        }
        if end == self.input.len() {
            return false;
        }
        let below = end + 1;
        self.caret = self.input[below..]
            .find('\n')
            .map_or(self.input.len(), |newline| below + newline);
        true
    }

    /// Move the caret to the line above, keeping its position along the line where it can.
    ///
    /// `false` when there is no line above, which is what leaves Up to the history it belongs to
    /// on a line with nothing to move within.
    pub fn move_up_a_line(&mut self) -> bool {
        let (start, _) = self.caret_line();
        if start == 0 {
            return false;
        }
        let above = self.input[..start - 1]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        self.caret = along(&self.input[above..start - 1], self.column()) + above;
        true
    }

    /// Move the caret to the line below, keeping its position along the line where it can.
    pub fn move_down_a_line(&mut self) -> bool {
        let (_, end) = self.caret_line();
        if end == self.input.len() {
            return false;
        }
        let below = end + 1;
        let ends = self.input[below..]
            .find('\n')
            .map_or(self.input.len(), |newline| below + newline);
        self.caret = along(&self.input[below..ends], self.column()) + below;
        true
    }

    /// How many characters along its line the caret is.
    fn column(&self) -> usize {
        let (start, _) = self.caret_line();
        self.input[start..self.caret].chars().count()
    }

    /// Delete the character after the caret, or the whole marker the caret is on.
    ///
    /// Whole for the reason [`Session::backspace`] takes one whole: the caret rests on a marker
    /// as it rests on a character, and half a marker stands for nothing.
    pub fn delete_forward(&mut self) {
        if self.caret == self.input.len() {
            return;
        }
        self.history.leave();
        match self.marker_at_caret() {
            Some((start, end)) => self.input.replace_range(start..end, ""),
            None => {
                self.input.remove(self.caret);
            }
        }
        self.completion = 0;
    }

    /// Delete the word before the caret.
    pub fn delete_word_before(&mut self) {
        self.history.leave();
        let was = self.caret;
        self.move_word_left();
        self.input.replace_range(self.caret..was, "");
        self.completion = 0;
    }

    /// Delete from the caret back to the start of its line.
    pub fn delete_to_line_start(&mut self) {
        self.history.leave();
        let (start, _) = self.caret_line();
        self.input.replace_range(start..self.caret, "");
        self.caret = start;
        self.completion = 0;
    }

    /// Delete from the caret to the end of its line.
    pub fn delete_to_line_end(&mut self) {
        self.history.leave();
        let (_, end) = self.caret_line();
        self.input.replace_range(self.caret..end, "");
        self.completion = 0;
    }

    /// What the half-typed line could still become: a command, or a file reference.
    ///
    /// One of the two at most. A command is the whole line and a reference is its last word, so
    /// nothing can be both.
    pub fn offered(&self) -> Offered {
        // First, before either guard below. The list of keys is documentation somebody asked for by
        // pressing a key, not machinery for finishing the line, so neither a turn in flight nor a
        // command line has anything to say about it. It takes the place of what is offered, since
        // the two answer the same space and only one of them was asked for.
        //
        // Answered after the guards, the flag was set by the press and the list had nowhere to go,
        // so `?` during a turn did nothing on screen and then put the list up unasked when the turn
        // ended. A turn going somewhere the person did not want is when they most want a key.
        if self.shortcuts {
            return Offered::Shortcuts;
        }
        // A line being composed while a turn runs is one Enter will queue, and what is offered for
        // it is machinery for finishing something about to be sent, which is the one thing a
        // running turn refuses.
        if self.status == Status::Working {
            return Offered::Nothing;
        }
        // A command line is neither a slash command nor a sentence with a file reference in it.
        // `/usr/bin/env` and an address with an `@` in it are ordinary arguments here, and
        // completing either would rewrite the line under someone typing a path.
        if self.shell {
            return Offered::Nothing;
        }
        let commands = crate::app::completions(&self.input);
        if !commands.is_empty() {
            return Offered::Commands(commands);
        }
        match crate::entries::typed_reference(&self.input) {
            Some(typed) => {
                let entries = crate::entries::matching(&self.workspace, typed);
                if entries.is_empty() {
                    Offered::Nothing
                } else {
                    Offered::Files(entries)
                }
            }
            None => Offered::Nothing,
        }
    }

    /// The commands the half-typed line could still become.
    pub fn completions(&self) -> Vec<crate::app::Command> {
        match self.offered() {
            Offered::Commands(commands) => commands,
            _ => Vec::new(),
        }
    }

    /// Which offered command is under the cursor, or `None` when no command is offered.
    ///
    /// Clamped here rather than when the input changes, because the list is a function of the
    /// input: typing a letter can shorten it, and a cursor past the end would otherwise choose
    /// nothing at the moment Tab was pressed.
    pub fn highlighted_completion(&self) -> Option<crate::app::Command> {
        let offered = self.completions();
        if offered.is_empty() {
            return None;
        }
        Some(offered[self.completion.min(offered.len() - 1)])
    }

    /// Which offered file is under the cursor, or `None` when no file is offered.
    pub fn highlighted_entry(&self) -> Option<crate::entries::Entry> {
        let Offered::Files(entries) = self.offered() else {
            return None;
        };
        entries
            .get(self.completion.min(entries.len().saturating_sub(1)))
            .cloned()
    }

    /// Whether something is being offered, so the keys that walk the list belong to it.
    ///
    /// The shortcuts are not: there is nothing to choose among them, so Tab and the arrows keep
    /// meaning what they mean everywhere else while the list is up.
    pub fn is_completing(&self) -> bool {
        matches!(self.offered(), Offered::Commands(_) | Offered::Files(_))
    }

    /// Whether taking what is offered would change the line.
    ///
    /// What Enter turns on. Tab may be pressed on a finished word harmlessly, but Enter has to
    /// choose between completing and sending, and a prompt ending in `@README.md` is a finished
    /// sentence even though the word is still what the list is about. Completing there would leave
    /// a user pressing Enter twice to say something perfectly well formed.
    pub fn completion_would_change_the_line(&self) -> bool {
        match self.offered() {
            Offered::Nothing | Offered::Shortcuts => false,
            Offered::Commands(_) => self
                .highlighted_completion()
                .is_some_and(|command| command.name != self.input.trim()),
            Offered::Files(entries) => {
                let Some(typed) = crate::entries::typed_reference(&self.input) else {
                    return false;
                };
                // A name the user finished typing is a finished sentence, whatever the list
                // happens to be highlighting: `@test` names a file of its own while a `tests/`
                // beside it sorts above. Walking the list with the arrows is a choice among the
                // rows and still wins, which is why this asks the untouched cursor.
                if self.completion == 0
                    && entries
                        .iter()
                        .any(|entry| !entry.is_directory && entry.path == typed)
                {
                    return false;
                }
                self.highlighted_entry()
                    .is_some_and(|entry| typed != entry.path)
            }
        }
    }

    /// How many things are offered, which is what bounds the cursor that walks them.
    pub fn offered_count(&self) -> usize {
        match self.offered() {
            Offered::Commands(commands) => commands.len(),
            Offered::Files(entries) => entries.len(),
            Offered::Nothing | Offered::Shortcuts => 0,
        }
    }

    /// Move down what is offered, stopping at the end.
    pub fn next_completion(&mut self) {
        let last = self.offered_count().saturating_sub(1);
        self.completion = (self.completion + 1).min(last);
    }

    /// Move up what is offered, stopping at the top.
    pub fn previous_completion(&mut self) {
        self.completion = self.completion.saturating_sub(1);
    }

    /// Take what is under the cursor.
    ///
    /// A command replaces the whole line, since a command *is* the line. A file replaces only the
    /// half-typed reference, because the rest is the sentence it was written into.
    ///
    /// Neither adds a trailing space when there is more to type: a command expecting an argument
    /// gets one, and so does a file, while a directory does not, so the path can be typed onwards
    /// into it.
    pub fn accept_completion(&mut self) {
        match self.offered() {
            Offered::Commands(_) => {
                let Some(command) = self.highlighted_completion() else {
                    return;
                };
                let line = if command.argument.is_empty() {
                    command.name.to_string()
                } else {
                    format!("{} ", command.name)
                };
                self.set_input(line);
            }
            Offered::Files(_) => {
                let Some(entry) = self.highlighted_entry() else {
                    return;
                };
                // The `@` that opened the reference is the one at the head of the last word,
                // not the last `@` in the line. A file may have one in its name, and cutting
                // there rebuilds the line around a path nobody chose: `@logo@2` plus the
                // offered `logo@2x.png` becomes `@logo@logo@2x.png`.
                let start = self
                    .input
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace())
                    .map_or(0, |(at, c)| at + c.len_utf8());
                if !self.input[start..].starts_with('@') {
                    return;
                }
                let kept = self.input[..start].to_string();
                let trailing = if entry.is_directory { "" } else { " " };
                self.set_input(format!("{kept}@{}{trailing}", entry.path));
            }
            Offered::Nothing | Offered::Shortcuts => return,
        }
        self.completion = 0;
    }

    /// Fill the transcript from a conversation resumed off disk.
    ///
    /// What the model can see is what the user is shown, which is the honest thing to draw: a
    /// resumed session that displayed more than it had would invite the user to refer to
    /// something the model has no record of.
    ///
    /// `recalled` is what each turn left beneath it: the plan it worked to and what its gates
    /// decided, by turn number. Both go on the last thing that turn said, which is where a live
    /// turn puts them, so a resumed transcript reads the same as one that is still running. A
    /// turn that said nothing keeps them on the prompt, since the alternative is dropping the
    /// record of a turn that was refused before it could answer.
    pub fn replay(
        &mut self,
        conversation: &bravebot_agent::Conversation,
        title: &str,
        recalled: &crate::sessions::Recalled,
    ) {
        use bravebot_agent::conversation::Said;

        self.note(t!(session_resumed, title = title));

        // The last entry of each turn, which is where that turn's trail goes. Filled as the
        // transcript is built and applied afterwards, so a turn that spoke several times ends up
        // with one trail on its last line rather than a copy under each of them.
        let mut last_of_turn: std::collections::BTreeMap<usize, usize> = Default::default();
        for said in conversation.recounted() {
            match said {
                Said::User(text) => {
                    self.turns += 1;
                    self.transcript.push(Entry::user(text));
                }
                Said::Assistant(text) => self.transcript.push(Entry::assistant(text, Vec::new())),
                Said::Tool(line) => self.transcript.push(Entry::recalled_tool(line)),
            }
            // An assistant entry before any prompt belongs to no turn, so there is nothing whose
            // trail it could be carrying.
            if self.turns > 0 {
                last_of_turn.insert(self.turns, self.transcript.len() - 1);
            }
        }

        for (turn, index) in last_of_turn {
            if let Some(trail) = recalled.trails.get(&turn) {
                self.transcript[index].trail = trail.clone();
            }
            if let Some(todos) = recalled.todos.get(&turn) {
                self.transcript[index].todos = todos.clone();
            }
        }
    }

    /// Begin sweeping a selection where the button went down.
    ///
    /// Allowed while a turn runs: reading and copying what is already on the screen changes
    /// nothing about the turn, and a long turn is exactly when someone wants to.
    pub fn begin_selection(&mut self, row: u16, column: u16) {
        self.selection = Some(crate::select::Selection::started_at(row, column));
        self.copied = None;
    }

    /// Follow the pointer with the loose end of the selection.
    pub fn extend_selection(&mut self, row: u16, column: u16) {
        if let Some(selection) = &mut self.selection {
            selection.extend_to(row, column);
        }
    }

    /// Forget the selection, after a click that swept over nothing.
    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.copied = None;
    }

    /// Record what a copy took, for the line that reports it.
    pub fn note_copied(&mut self, characters: usize) {
        self.copied = Some(characters);
    }

    /// Insert pasted text into the input.
    ///
    /// Kept apart from typing because a paste is one act, not a stream of keys. Pasted text
    /// routinely ends in a newline, and a terminal that delivers a paste as keystrokes turns
    /// that into Enter: a prompt copied from somewhere else used to send itself before its
    /// author had read it back. Nothing here submits.
    ///
    /// Line endings are normalised so text copied from anywhere lands as the same thing. The
    /// newlines are kept rather than flattened, since a pasted paragraph was written with them
    /// and the box draws them.
    pub fn paste(&mut self, text: &str) {
        self.history.leave();
        let text = normalised(text);
        self.input.insert_str(self.caret, &text);
        self.caret += text.len();
    }

    /// Take text the user pasted, folding a long one behind a marker.
    ///
    /// [`Session::paste`] writes whatever it is given, which is what the markers themselves are
    /// written with, so the folding lives here: a paste is the one thing that arrives long enough
    /// to be worth hiding, and everything else that reaches the line is already a row or less.
    ///
    /// Long is counted in newlines rather than in rows the box would draw, because a wrapped line
    /// is one line the user pasted and folding on the width would fold differently in a narrow
    /// window. Two of them read fine in the box; the third is where a paste starts taking the
    /// screen, so that is where it is put away.
    ///
    /// Shell mode is left alone, and has to be: the line there is the command, and a command that
    /// is not what the user is looking at is the one thing that mode may never do.
    pub fn paste_text(&mut self, text: &str) {
        let text = normalised(text);
        if self.shell || text.matches('\n').count() < FOLD_AT_NEWLINES {
            self.paste(&text);
            return;
        }

        // Numbered off the counter a dropped file and a pasted picture use, so no two markers in
        // one line can carry the same number and a number is never reused.
        self.attachments_made += 1;
        let marker = t!(
            paste_folded,
            number = self.attachments_made,
            lines = lines_in(&text)
        );
        self.paste(&marker);
        self.pasted_text.push(PastedText { marker, text });
    }

    /// A line with every paste marker in it put back to the text it stands for.
    ///
    /// Every marker, not the ones a count says should be there: a user who deleted one meant to
    /// drop that paste, and one who copied a marker to somewhere else in the line meant the words
    /// twice.
    ///
    /// Called where the line leaves the box, so the words are what is sent, what the transcript
    /// keeps and what the history remembers. The box is the only place a marker belongs: it is
    /// there to keep a stack trace from taking the screen while somebody is typing around it, and
    /// a marker that outlived the line would be a handle on words this session is the only one
    /// holding.
    pub fn unfolded(&self, line: &str) -> String {
        let mut line = line.to_string();
        for pasted in &self.pasted_text {
            line = line.replace(&pasted.marker, &pasted.text);
        }
        line
    }

    /// A line with every paste in it put back behind the marker that stood for it.
    ///
    /// The inverse, for a line coming back to the box after a turn was stopped. What returns is
    /// the line as it was typed, because the box is where a long paste takes the screen and
    /// somebody who has just stopped a turn is about to edit the prompt, not read it.
    pub fn folded(&self, line: &str) -> String {
        let mut line = line.to_string();
        for pasted in &self.pasted_text {
            line = line.replace(&pasted.text, &pasted.marker);
        }
        line
    }

    /// A line in the form worth remembering, with every marker settled.
    ///
    /// A marker stands for something staged beside the line, and nothing staged outlives the
    /// session that staged it. Remembered as it stands, a marker comes back naming nothing: a
    /// picture nobody can produce, a file nothing read.
    ///
    /// A dropped file becomes its name, which is a path a person recognises and one the planner
    /// can go and read through the gate it reads anything through. That is what a file dropped
    /// onto a line queued mid-turn already becomes.
    ///
    /// A picture becomes nothing at all, along with the space beside it, because there is no
    /// durable text that stands for a screenshot and the words around it are what the person
    /// meant. Somebody recalling that prompt pastes the picture they mean now.
    fn recallable(&self, line: &str) -> String {
        let mut line = line.to_string();
        for attached in &self.attached {
            line = line.replace(&attached.marker, &attached.name);
        }
        for pasted in &self.pasted {
            line = line.replace(&format!("{} ", pasted.marker), "");
            line = line.replace(&format!(" {}", pasted.marker), "");
            line = line.replace(&pasted.marker, "");
        }
        line
    }

    /// Take a paste that turned out to be a drop, or say it was not one.
    ///
    /// A recognised file becomes a marker in the line and an attachment behind it. Anything else,
    /// an unsupported type or a path naming no file at all, has its path written out, which is
    /// what dropping a file did before any of this existed.
    ///
    /// Returns whether the text was a drop at all. A paste that was not one is left to
    /// [`Session::paste`], untouched.
    pub fn drop_files(&mut self, text: &str) -> bool {
        let exists = |path: &str| std::path::Path::new(path).is_file();
        if !crate::dropped::is_drop(text, exists) {
            return false;
        }

        let taken = crate::dropped::dropped_with(text, exists);
        let mut written = Vec::new();

        for path in crate::dropped::paths(text) {
            match taken
                .iter()
                .find(|found| found.path == path)
                .and_then(|found| {
                    crate::dropped::name_for(&self.workspace, &found.path).map(|name| (found, name))
                }) {
                Some((found, name)) => {
                    self.attachments_made += 1;
                    let marker = format!("[{} #{}]", found.noun(), self.attachments_made);
                    self.attached.push(Attached {
                        marker: marker.clone(),
                        name,
                        shown: found.path.clone(),
                        kind: found.kind,
                    });
                    written.push(marker);
                }
                // Out of reach, or a type nothing here takes. The path is what a drop always
                // produced, and it is still useful: the user can read it and say what they meant.
                None => written.push(path),
            }
        }

        // A trailing space, which is what a terminal does when a file is dropped into a shell:
        // whatever is typed next, or dropped next, does not run into the marker.
        self.paste(&format!("{} ", written.join(" ")));
        true
    }

    /// The attachments the line still names, in the order they appear in it.
    ///
    /// Read back out of the line rather than taken wholesale, so deleting a marker takes its
    /// attachment off. That is the only way a user has to change their mind, since the marker is
    /// the only part of it they can see.
    pub fn attachments_named(&self, line: &str) -> Vec<Attached> {
        self.named_in(line).cloned().collect()
    }

    /// What the line being typed carries, for drawing under the box.
    ///
    /// The same question the turn is built from asks, so the row and the turn can never disagree
    /// about which files a line is carrying. Drawn from what a drop staged instead, a file whose
    /// marker the person had rubbed out kept its row, which is the one place they can see whether
    /// rubbing it out worked.
    pub fn attached_to_the_line(&self) -> impl Iterator<Item = &Attached> {
        self.named_in(&self.input)
    }

    /// Everything a drop staged, whether or not the line still names it.
    pub fn attached(&self) -> &[Attached] {
        &self.attached
    }

    /// The attachments a line names, without cloning them.
    fn named_in<'a>(&'a self, line: &'a str) -> impl Iterator<Item = &'a Attached> {
        self.attached
            .iter()
            .filter(move |attached| line.contains(&attached.marker))
    }

    /// What the line carried when it was sent, for the task being built from it.
    pub fn sent_attachments(&self) -> &[Attached] {
        &self.sent
    }

    /// Attach a pasted picture, writing the text that stands for it where the caret is.
    ///
    /// The marker is what makes a picture something a person can see and edit. Without one the
    /// prompt would say nothing about what came with it, and a user would be left counting pastes
    /// to work out what the planner was about to be shown.
    ///
    /// Numbered off the same counter a dropped file uses, so a paste and a drop can never both
    /// call themselves `[Image #1]`, and so a number is never reused: renumbering on a deletion
    /// would change the marker sitting in the line the user is looking at.
    pub fn attach(&mut self, image: crate::clipboard::Image) {
        self.attachments_made += 1;
        // Not from a catalog, and deliberately. Unlike a folded paste, which is put back to its
        // words before the turn is built, this marker is sent as it stands: the planner reads
        // "[Image #2]" and counts to the picture that answers it. Translating it would change
        // what the model is given, which is the one thing a change of language must not do.
        let marker = format!("[Image #{}]", self.attachments_made);
        self.paste(&marker);
        self.pasted.push(AttachedImage {
            marker,
            media_type: image.media_type,
            bytes: image.bytes,
        });
    }

    /// The pictures the line still names, in the order they appear in it.
    ///
    /// Read back out of the line for the reason [`Session::attachments_named`] is: deleting the
    /// marker is the only way a user has to take a picture back off.
    pub fn pasted_named(&self, line: &str) -> Vec<AttachedImage> {
        self.pasted
            .iter()
            .filter(|pasted| line.contains(&pasted.marker))
            .cloned()
            .collect()
    }

    /// How many pictures the line being typed still refers to, for the line beneath the box.
    pub fn pasted_count(&self) -> usize {
        self.pasted_named(&self.input).len()
    }

    /// What the line carried when it was sent, for the task being built from it.
    pub fn sent_pasted(&self) -> &[AttachedImage] {
        &self.sent_pasted
    }

    /// Every marker standing in the line for something carried beside it.
    fn markers(&self) -> impl Iterator<Item = &str> {
        self.attached
            .iter()
            .map(|attached| attached.marker.as_str())
            .chain(self.pasted.iter().map(|pasted| pasted.marker.as_str()))
            .chain(self.pasted_text.iter().map(|pasted| pasted.marker.as_str()))
    }

    /// Where every marker in the line starts and ends.
    ///
    /// Found by looking the line up rather than by remembering a position, because the line is
    /// edited around a marker and a remembered offset would be wrong the first time somebody
    /// rewrote the sentence in front of it.
    fn marker_spans(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.markers().flat_map(|marker| {
            self.input
                .match_indices(marker)
                .map(|(at, found)| (at, at + found.len()))
        })
    }

    /// The marker the caret would move back into, if it is against one.
    ///
    /// A caret at the very start of a marker is not against it from this side: what sits before
    /// that caret is ordinary text, and moving over it or deleting it is an ordinary press.
    fn marker_before_caret(&self) -> Option<(usize, usize)> {
        self.marker_spans()
            .find(|&(start, end)| start < self.caret && self.caret <= end)
    }

    /// The marker the caret is on, meaning the one the next forward press would move over.
    ///
    /// A caret at the end of a marker is past it, and what lies ahead is the text after it.
    pub fn marker_at_caret(&self) -> Option<(usize, usize)> {
        self.marker_spans()
            .find(|&(start, end)| start <= self.caret && self.caret < end)
    }

    /// Delete the character before the caret, or leave shell mode where there is nothing left to
    /// delete.
    ///
    /// Deleting back past the `!` leaves the mode, which is where the marker appears to be: a user
    /// who typed it by mistake gets rid of it the way they got rid of any other character. Without
    /// this the mode could only be left by clearing the whole line.
    ///
    /// A marker goes whole. It is one thing on the screen and one thing to the user, so taking a
    /// character off the end of it would leave text standing for a picture that is no longer
    /// attached, and the user would have to keep pressing to find out.
    ///
    /// A marker the caret is covering goes first, before anything in front of it. The covering is
    /// visible: the whole of that marker is drawn under the caret, and a press that took the
    /// character beside it instead would take something the user could see was not selected.
    pub fn backspace(&mut self) {
        self.history.leave();
        if let Some((start, end)) = self
            .marker_at_caret()
            .or_else(|| self.marker_before_caret())
        {
            self.input.replace_range(start..end, "");
            self.caret = start;
            self.completion = 0;
            return;
        }
        // Nothing before the caret is where the marker appears to be, so this is the press that
        // deletes it. Whatever follows stays and becomes an ordinary prompt: the mode is what was
        // deleted, not the words.
        if self.caret == 0 {
            self.shell = false;
            return;
        }
        self.move_left();
        self.input.remove(self.caret);
        self.completion = 0;
    }

    /// Settle a turn that was stopped: either un-sent whole, or recorded as having stopped.
    ///
    /// The text returns to the box so a user who changed their mind can adjust it rather than
    /// retype it, which is the whole point of cancelling rather than waiting. Two things stop
    /// that, and both mean the prompt stays sent: work that is already on the screen, and prompts
    /// waiting behind this one.
    pub fn restore(&mut self, prompt: impl Into<String>) {
        self.status = Status::Idle;
        self.started = None;
        self.phase = None;
        self.running = None;
        self.scroll = 0;
        // A prompt is English and a command line is not, so the line coming back must not land
        // behind a marker that would run it. Belt and braces with the guard in
        // [`Session::type_char`]: this is the state the returning text lands in, and it has to be
        // safe whatever left the mode armed.
        self.shell = false;

        // Un-sent whole only where nothing was recorded after the prompt and nothing is waiting
        // behind it. Either one means there is something to have second thoughts about.
        let waiting = !self.queued.is_empty();
        if waiting
            || !matches!(self.transcript.last(), Some(entry) if entry.speaker == Speaker::User)
        {
            // The turn visibly did things, some of which touched the workspace. Putting the
            // prompt back would offer to redo work that is on the screen, and removing what
            // happened would hide it, so both stay and the stop is recorded.
            //
            // Or the person has queued more prompts, and the next of them is about to go. The
            // box belongs to what they type next, not to a line they sent before the two that
            // are still to run, and the conversation has to read in the order it happened.
            let todos = std::mem::take(&mut self.todos);
            self.transcript
                .push(Entry::system("stopped").with_todos(todos));
            return;
        }

        self.transcript.pop();
        // Popped because the text is going back into the box: offering it from history as well
        // would present the same line from two places. Rewritten rather than appended, since the
        // stored copy has to go too.
        self.history.pop();
        if self.persist {
            crate::store::save_history(self.history.entries());
        }
        // Only where the box is empty. A user who typed while the turn ran meant those words,
        // and putting the old prompt over the top of them would lose the newer of the two.
        if self.input.trim().is_empty() {
            // Back the way it was typed. A paste that returned as its words would fill the box
            // somebody is about to edit with the stack trace they folded away in the first place.
            let returning = self.folded(&prompt.into());
            self.set_input(returning);
            // The pictures come back with the words. A line that returned without them would
            // return carrying markers that name nothing, and the user has no way to tell.
            self.pasted = std::mem::take(&mut self.sent_pasted);
        }
        // Discarded rather than kept: the prompt is going back into the box as though it had never
        // been sent, so a plan for a turn that is being un-sent has nothing to describe.
        self.todos.clear();
    }

    /// Discard whatever has been typed.
    ///
    /// Guarded like the other editing methods: input belongs to the idle state, and clearing it
    /// mid-turn would mean the field the user returns to is not the one they left.
    pub fn clear_input(&mut self) {
        if self.status == Status::Idle {
            self.history.leave();
            self.set_input(String::new());
            // The mode goes with the line. Escape means "never mind this", and leaving the marker
            // behind would arm the next thing typed as a command. The list of keys goes with the
            // line too, which is settled where the line is written.
            self.shell = false;
        }
    }

    /// Put the line away, or bring back the one that was put away.
    ///
    /// Which of the two it does is read off the line rather than chosen: a line to put away is put
    /// away, and an empty box is where a line put away earlier is wanted. Returns whether anything
    /// happened, so a press that had nothing to do either way can be told from one that acted.
    ///
    /// The words alone travel. The mode stays where the user left it, so a prompt put away as a
    /// prompt comes back into an armed shell as the command they meant to write, and a caret is not
    /// carried because it belongs to an edit that has finished. Bringing one back empties the slot:
    /// the line is in the box now, and a second press would put a copy of it beside the first.
    ///
    /// Markers are not touched on the way out. What is staged stays staged, and the row beneath the
    /// box goes on saying so because it is drawn from what the line names: a marker put away names
    /// nothing until the words holding it come back, and then it names what it always did.
    ///
    /// Allowed while a turn runs, exactly as typing and recall are: this writes a line and sends
    /// nothing, and sending is the whole of what a running turn refuses. A turn in flight is when a
    /// person most wants a half-written thought out of the way, since it is when a better one has
    /// just occurred to them.
    pub fn stash(&mut self) -> bool {
        self.history.leave();
        // The list of keys is not part of the line and cannot be put away, but it is standing over
        // a box that is about to change, and everything else that rewrites the box takes it down.
        self.shortcuts = false;
        self.completion = 0;

        if self.input.is_empty() {
            // Taken rather than read, so the slot empties as the line fills: what was put away is
            // back in front of the user, and the only copy of it is the one they can see.
            match self.stashed.take() {
                Some(line) => {
                    self.set_input(line);
                    true
                }
                None => false,
            }
        } else {
            // Overwriting rather than stacking. One slot is what the key promises, and a press that
            // silently pushed a second line would leave the first reachable only by pressing again.
            self.stashed = Some(std::mem::take(&mut self.input));
            self.caret = 0;
            true
        }
    }

    /// The line put away, for saying that there is one.
    pub fn stashed(&self) -> Option<&str> {
        self.stashed.as_deref()
    }

    /// Take the line back from an external editor.
    ///
    /// Replaces rather than appends: the editor was opened on this line, so what comes back is
    /// the same line after however much thought, not something to add to it.
    ///
    /// Guarded like the other editing methods. Nothing can reach it mid-turn, since the key is
    /// ignored while one runs, but the field belongs to the idle state either way.
    pub fn take_edited(&mut self, line: impl Into<String>) {
        if self.status != Status::Idle {
            return;
        }
        // A recalled prompt that has been through an editor is the working line now, exactly as
        // it would be after a keystroke.
        self.history.leave();
        self.set_input(line);
        self.completion = 0;
    }

    /// Show the previous prompt, stepping further back on each call.
    ///
    /// Allowed while a turn runs, exactly as typing into the box is. Recall writes a line and
    /// sends nothing, and sending is the whole of what a running turn refuses. It used to refuse
    /// here too, from the days when the box took nothing mid-turn at all; typing was opened up
    /// and this was left behind, so a person could compose their next prompt during a turn but
    /// not reach the one they sent last.
    pub fn recall_older(&mut self) {
        if let Some(prompt) = self.history.older(&self.input) {
            self.set_input(prompt);
        }
    }

    /// Step forward through recalled prompts, back to the line being typed.
    pub fn recall_newer(&mut self) {
        if let Some(prompt) = self.history.newer() {
            self.set_input(prompt);
        }
    }

    /// Take the current line as a command to run, if shell mode is on and there is one.
    ///
    /// Leaves shell mode, so the next line is a prompt again: the mode lasts for one command, the
    /// way it does in the interface this follows. The line is recorded in the prompt history, since
    /// a command someone ran is a line they may well want back.
    pub fn submit_command(&mut self) -> Option<String> {
        if self.status != Status::Idle || !self.shell {
            return None;
        }
        let line = self.input.trim().to_string();
        if line.is_empty() {
            return None;
        }
        self.set_input(String::new());
        self.shell = false;
        self.completion = 0;
        self.remember(&line);
        self.transcript.push(Entry::shell(line.clone()));
        self.scroll = 0;
        Some(line)
    }

    /// Record a prompt as sent, in this session and on disk.
    ///
    /// One place rather than three, because a prompt reaches the history from three of them and
    /// what is recorded beside it has to be the same each time. Nothing is written for a prompt
    /// the history collapsed into the one before it: the file would hold the second copy that the
    /// session in front of the person does not.
    ///
    /// What is written is the line with its markers settled, because the session that recalls it
    /// staged none of them.
    fn remember(&mut self, prompt: &str) {
        let project = self.project();
        let stored = self.history.push(self.recallable(prompt), project).cloned();
        if let (true, Some(entry)) = (self.persist, stored) {
            crate::store::append_history(&entry);
        }
    }

    /// The workspace a prompt is recorded as having been sent from.
    ///
    /// `None` for a session built without one, which is every session in a test: a prompt recorded
    /// against the process's working directory would file the suite's prompts under whatever
    /// directory cargo happened to run in.
    fn project(&self) -> Option<String> {
        match self.workspace.as_os_str().is_empty() {
            true => None,
            false => Some(self.workspace.display().to_string()),
        }
    }

    /// The spinner glyph for this moment, for a command in flight.
    pub fn spinner(&self) -> &'static str {
        crate::indicator::glyph_at(self.elapsed())
    }

    /// How long the command in flight has been running, in the words the indicator uses.
    pub fn elapsed_words(&self) -> String {
        crate::indicator::format_elapsed(self.elapsed())
    }

    /// Note that a command is running, so the box shows it rather than an empty prompt.
    pub fn begin_command(&mut self) {
        self.status = Status::Running;
        self.started = Some(Instant::now());
        self.scroll = 0;
    }

    /// Note that it finished, whatever came of it.
    pub fn finish_command(&mut self) {
        self.status = Status::Idle;
        self.started = None;
    }

    /// Show what a command printed, or that it printed nothing.
    pub fn printed(&mut self, text: &str) {
        if text.trim().is_empty() {
            self.transcript.push(Entry::system(t!(session_no_output)));
        } else {
            self.transcript.push(Entry::output(text.trim_end()));
        }
        self.scroll = 0;
    }

    /// Take the current input as a prompt, if there is one.
    ///
    /// Clears the field and records the prompt in the transcript, so the display reflects
    /// the submission even before a reply arrives.
    ///
    /// A folded paste is put back to its words here, which is where the line stops being
    /// something being typed and becomes something that was sent.
    pub fn submit(&mut self) -> Option<String> {
        if self.status != Status::Idle {
            return None;
        }
        let typed = self.input.trim().to_string();
        if typed.is_empty() {
            return None;
        }
        let prompt = self.unfolded(&typed);
        // Recorded here rather than in `begin_turn`, because a queued prompt was recorded when it
        // was queued: from the person's side that is when they sent it. Before the line is taken,
        // because taking it clears what the markers in it stand for.
        self.remember(&prompt);
        // Settled from the line as it was typed, since that is where the markers are. Everything
        // still named goes; a marker the user deleted is an attachment they took off, and it goes
        // nowhere. Pictures settle the same way and at the same moment, since a marker rubbed out
        // means the same thing whether the thing behind it was dropped or pasted.
        let taken = self.take_line(&typed);
        Some(self.begin_turn(prompt, taken))
    }

    /// Take the current line as a prompt to send when the turn in flight has finished.
    ///
    /// The line leaves the box exactly as it would on sending, and is remembered in the history
    /// the same way, because from the person's side they have sent it. What has not happened yet
    /// is the turn, so nothing goes into the transcript until this one's own turn begins.
    ///
    /// Only while a turn is running. With none there is nothing to wait for and
    /// [`Session::submit`] is what Enter means.
    pub fn queue(&mut self) -> bool {
        if self.status != Status::Working {
            return false;
        }
        let typed = self.input.trim().to_string();
        if typed.is_empty() {
            return false;
        }
        // Resolved before the line is taken, because taking it clears what the markers stand for.
        let resolved = self.resolved(&typed);
        let prompt = self.unfolded(&typed);
        self.remember(&prompt);
        let (attached, pasted) = self.take_line(&typed);
        // Into the turn's reach the moment it is typed, rather than when the turn next asks. The
        // turn asks between rounds and a round can be a long wait; put there now, the line is
        // taken at the first boundary after it was sent, which is the soonest anything could act
        // on it.
        //
        // The resolved copy lives only there. What is kept here is what the person typed, because
        // that is what the screen and the history are for, and a second copy of the resolved line
        // would be one for the two to disagree over.
        self.pending.push(resolved);
        self.queued.push(Queued {
            prompt,
            attached,
            pasted,
        });
        self.scroll = 0;
        true
    }

    /// The buffer the running turn takes interjections from.
    ///
    /// Handed to the worker when a turn starts. Cloning shares the buffer rather than copying it,
    /// which is the point: what the interface adds is what the turn takes.
    pub fn interjections(&self) -> crate::remote_confirm::Interjections {
        self.pending.clone()
    }

    /// A queued line with every marker in it put back to words.
    ///
    /// What a mid-turn prompt is given to the planner as. A turn already running cannot be handed
    /// a file or a picture: routing was precommitted before it read anything, and a file admitted
    /// now would be context this turn never fixed the shape of. So what travels is text, and each
    /// marker becomes the most honest sentence available about what stood there.
    ///
    /// A paste becomes the words themselves, exactly as [`Session::unfolded`] makes it: the marker
    /// was only ever a way of drawing a long paste short.
    ///
    /// A file and a picture cannot become their contents, so they become their names. A named file
    /// is one the planner can go and read, through the same gate it reads anything else through,
    /// and a person who dropped a file mid-turn is telling it which file to look at. A picture has
    /// no such recourse and says so: better a planner that knows a screenshot was meant for it and
    /// cannot see it than one that is handed `[Image #2]` and left counting.
    fn resolved(&self, line: &str) -> String {
        let mut resolved = self.unfolded(line);
        for attached in self.named_in(line) {
            resolved = resolved.replace(&attached.marker, &attached.name);
        }
        // Not from the catalog, and deliberately: the planner reads this, so it is part of what the
        // model is given rather than something a person is being told. Same reasoning as the marker
        // in [`Session::attach`], and the catalog says so at the top of itself.
        for pasted in self.pasted_named(line) {
            resolved = resolved.replace(
                &pasted.marker,
                &format!(
                    "({} was pasted here, but it cannot be shown to you: a picture cannot join a \
                     turn already running. Ask for it again if you need to see it.)",
                    pasted.marker
                ),
            );
        }
        resolved
    }

    /// Record that the prompt queued longest ago has reached the planner mid-turn.
    ///
    /// Called when the turn says it took one, rather than when the line was put where the turn
    /// could reach it: until it is taken the prompt is still waiting, and it belongs above the box
    /// where a waiting prompt is drawn. This is the moment it becomes part of the conversation, so
    /// this is the moment it joins the transcript, which reads in the order things happened.
    pub fn interjected(&mut self) {
        if self.queued.is_empty() {
            return;
        }
        let gone = self.queued.remove(0);
        self.transcript.push(Entry::user(gone.prompt));
        self.scroll = 0;
    }

    /// Begin the turn for the prompt queued longest ago, if the session is free to start one.
    ///
    /// What a prompt still waiting when the turn ended does, which is what every queued prompt used
    /// to do. It becomes a turn of its own, and as its own turn it gets what a turn gets: routing
    /// precommitted from it, and the files and pictures it named carried with it. That is why one
    /// left over is better off here than interjected, and why nothing tries to hurry it.
    pub fn send_queued(&mut self) -> Option<String> {
        if self.status != Status::Idle || self.queued.is_empty() {
            return None;
        }
        let next = self.queued.remove(0);
        // The copy left for the running turn goes: this prompt is becoming a turn of its own, and a
        // copy still in the buffer would reach the planner a second time, as an interjection into
        // the very turn this line started.
        self.pending.take();
        Some(self.begin_turn(next.prompt, (next.attached, next.pasted)))
    }

    /// The loop repeating a prompt, where one is running.
    pub fn looping(&self) -> Option<&crate::loops::Running> {
        self.looping.as_ref()
    }

    /// Start repeating a prompt, and give back the first tick to send now.
    ///
    /// The first tick goes immediately rather than after a wait. Somebody who has just asked for
    /// something every five minutes wants to see it happen, and a loop whose first sign of life
    /// is five minutes of nothing is one nobody can tell is running.
    ///
    /// `None` where the session is not free to start a turn, since the first tick is a turn like
    /// any other and there is nowhere to put it.
    pub fn start_loop(&mut self, request: crate::loops::Request) -> Option<String> {
        if self.status != Status::Idle {
            self.note(t!(loop_busy));
            return None;
        }
        if self.looping.is_some() {
            self.note(t!(loop_replaced));
        }

        match request.adjusted {
            Some(crate::loops::Held::Raised(every)) => {
                self.note(t!(loop_interval_raised, every = crate::loops::spell(every)))
            }
            Some(crate::loops::Held::Capped(every)) => {
                self.note(t!(loop_interval_capped, every = crate::loops::spell(every)))
            }
            None => {}
        }
        match request.pacing {
            crate::loops::Pacing::Every(every) => {
                self.note(t!(loop_started_every, every = crate::loops::spell(every)))
            }
            crate::loops::Pacing::SelfPaced => self.note(t!(loop_started_self_paced)),
        }

        self.looping = Some(crate::loops::Running::begin(request));
        self.dispatch_tick()
    }

    /// Stop the loop, and say whether there was one.
    pub fn stop_loop(&mut self) -> bool {
        let stopped = self.looping.take().is_some();
        if stopped {
            self.note(t!(loop_stopped));
        }
        stopped
    }

    /// Send the next tick, if one is due and the session is free to take it.
    ///
    /// Called on the way round the interface's own loop, so a tick waits for the turn in flight
    /// and for everything the person queued behind it. A schedule is a request to be asked
    /// again, not a licence to interrupt.
    pub fn loop_tick(&mut self) -> Option<String> {
        let now = Instant::now();
        let running = self.looping.as_ref()?;
        if running.aged_out(now) {
            self.looping = None;
            self.note(t!(loop_aged_out));
            return None;
        }
        if self.status != Status::Idle || !self.queued.is_empty() || !running.due(now) {
            return None;
        }
        self.dispatch_tick()
    }

    /// Arm the next tick from the turn that has just ended.
    ///
    /// `wakeup` is what a self-paced turn asked for. Nothing happens where the finished turn was
    /// not a tick: a person typing their own prompt in the middle of a loop is not a tick of it,
    /// and must not reset the clock.
    pub fn loop_turn_ended(&mut self, wakeup: Option<crate::loops::Wakeup>) {
        let Some(running) = self.looping.as_mut() else {
            return;
        };
        if !running.ended(wakeup, Instant::now()) {
            self.looping = None;
            self.note(t!(loop_unpaced));
        }
    }

    /// Send a tick, announcing which one it is.
    ///
    /// The line is the loop's own, taken from what the person typed, and it carries no files or
    /// pictures: what was staged in the box belonged to the line it was staged in. A `@path` in
    /// the prompt is read back out of it every tick, the way it is for any other prompt.
    fn dispatch_tick(&mut self) -> Option<String> {
        let running = self.looping.as_mut()?;
        running.dispatched();
        let count = running.ticks();
        let quiet = running.quiet();
        let prompt = running.prompt().to_string();
        if quiet > 0 {
            self.note(t!(loop_tick_quiet, count = count, quiet = quiet));
        } else {
            self.note(t!(loop_tick, count = count));
        }
        Some(self.begin_turn(prompt, (Vec::new(), Vec::new())))
    }

    /// Take every waiting prompt back out of the queue and into the box.
    ///
    /// All of them, in the order they were typed, one to a line, and the half-written line already
    /// in the box stays below them: it was typed after they were, and it is where the caret goes,
    /// so somebody carries on where they left off. What each of them named comes back with it, the
    /// way a stashed line's markers do, since a marker with nothing behind it would send a prompt
    /// pointing at a file that is no longer staged.
    ///
    /// The prompts stay in the history. From the person's side they were sent, and taking them back
    /// does not unsay them.
    pub fn unqueue(&mut self) -> bool {
        if self.queued.is_empty() {
            return false;
        }
        // From the back, which is the end this key takes from, and only as far as the turn has not
        // already reached. A prompt comes back only if its copy was still there to drop: a line the
        // planner has been given cannot be unsaid, and offering it back to the box would leave the
        // person editing a prompt that had already gone. The two threads move independently, so a
        // line can be taken between the key press and this loop.
        //
        // Before anything is disturbed, so that finding nothing left to take leaves the box exactly
        // as it was rather than half rewritten.
        let mut reclaimed = Vec::new();
        while !self.queued.is_empty() && self.pending.forget_last() {
            reclaimed.push(self.queued.pop().expect("the queue was not empty"));
        }
        if reclaimed.is_empty() {
            return false;
        }

        // The box is about to be rewritten, so the things standing over it that belong to the line
        // it held go, exactly as they do for anything else that writes a whole line.
        self.history.leave();
        self.completion = 0;

        let mut lines = Vec::new();
        let mut attached = Vec::new();
        let mut pasted = Vec::new();
        for waiting in reclaimed.into_iter().rev() {
            lines.push(waiting.prompt);
            attached.extend(waiting.attached);
            pasted.extend(waiting.pasted);
        }
        if !self.input.is_empty() {
            lines.push(std::mem::take(&mut self.input));
        }
        attached.append(&mut self.attached);
        self.attached = attached;
        pasted.append(&mut self.pasted);
        self.pasted = pasted;
        self.set_input(lines.join("\n"));
        true
    }

    /// Clear the line and settle what it named.
    ///
    /// Everything still named goes; a marker the user deleted is an attachment they took off, and
    /// it goes nowhere. Pictures settle the same way and at the same moment, since a marker rubbed
    /// out means the same thing whether the thing behind it was dropped or pasted.
    fn take_line(&mut self, prompt: &str) -> (Vec<Attached>, Vec<AttachedImage>) {
        let attached = self.attachments_named(prompt);
        self.attached.clear();
        let pasted = self.pasted_named(prompt);
        self.pasted.clear();
        self.set_input(String::new());
        (attached, pasted)
    }

    /// Start a turn for a prompt, whether it was sent just now or waited for its turn.
    fn begin_turn(&mut self, prompt: String, taken: (Vec<Attached>, Vec<AttachedImage>)) -> String {
        (self.sent, self.sent_pasted) = taken;
        self.transcript.push(Entry::user(prompt.clone()));
        self.status = Status::Working;
        self.scroll = 0;
        self.turns += 1;
        // The last turn's figures are not this one's, and a line reporting a finished turn while
        // another is running is a line about the wrong turn.
        self.finished = None;
        // The previous turn's plan is not this turn's. Leaving it would show finished work as
        // though the new turn had it outstanding.
        self.todos.clear();
        self.written = 0;
        self.phase = None;
        self.running = None;
        self.started = Some(Instant::now());
        prompt
    }

    /// Record a completed turn, and what it cost.
    pub fn complete(&mut self, reply: impl Into<String>, trail: Vec<Event>, tokens: u64) {
        let trail = trail.iter().map(crate::audit::as_line).collect();
        // The list moves onto the entry rather than being dropped, so what the turn set out to do
        // stays in the scrollback next to the answer it produced.
        let todos = std::mem::take(&mut self.todos);
        let reply = reply.into();
        self.transcript
            .push(Entry::assistant(crate::reasoning::spoken(&reply), trail).with_todos(todos));
        self.status = Status::Idle;
        self.scroll = 0;
        self.finished = Some(Finished {
            turn: self.turns,
            tokens,
            took: self.elapsed(),
            failed: false,
        });
        self.started = None;
        self.phase = None;
        self.running = None;
        self.streaming.clear();
        // Accumulated across the session: the figure answers "what has this cost me", which is
        // about the session rather than the last turn.
        self.tokens += tokens;
        // Added to rather than set, since a turn that compacted part way through has already put
        // that cost here under the same number.
        *self.spend.entry(self.turns).or_insert(0) += tokens;
        // The wall clock is taken here because this is the last moment it can be: the timer is
        // cleared two lines up. It is also the better of the two available figures. The turn loop
        // measures its own span, but a turn runs on a worker and the person was waiting from the
        // moment they pressed Enter, which is what this clock started on.
        //
        // Recorded whether or not the breakdown ever arrives, so a turn always accounts for its own
        // wall clock and a missing breakdown reads as time nothing has claimed rather than as a turn
        // that never happened. See [`Session::spent_time`].
        self.timing.entry(self.turns).or_default().wall_ms += self
            .finished
            .map_or(0, |f| u64::try_from(f.took.as_millis()).unwrap_or(u64::MAX));
    }

    /// Record how the turn just finished divided its time up.
    ///
    /// Beside [`Session::complete`] rather than inside it, in the same way the context measurement
    /// is: both are facts the worker brings back about the turn, and neither is knowable from the
    /// transcript. The wall figure is not taken from here, because this side has the better one.
    ///
    /// Added to rather than set, for the reason the token breakdown is: a turn that compacted part
    /// way through has already charged that wait to the same number.
    pub fn spent_time(&mut self, timing: bravebot_agent::timing::Timing) {
        let entry = self.timing.entry(self.turns).or_default();
        entry.inference_ms += timing.inference_ms;
        entry.tools_ms += timing.tools_ms;
        entry.stalled_ms += timing.stalled_ms;
    }

    /// Record a failure. The turn is over either way, so the session returns to idle.
    ///
    /// The list is kept on the entry as it stood, unfinished. A failed turn that had got three of
    /// five tasks done is more useful shown that way than blank.
    pub fn fail(&mut self, message: impl Into<String>) {
        let todos = std::mem::take(&mut self.todos);
        self.transcript
            .push(Entry::system(message).with_todos(todos));
        self.status = Status::Idle;
        self.scroll = 0;
        // Reported as a failure rather than left to the success line, which would put a tick
        // beside a turn that did not finish.
        self.finished = Some(Finished {
            turn: self.turns,
            tokens: 0,
            took: self.elapsed(),
            failed: true,
        });
        // A turn that failed after ten minutes still spent them, and it is the turn most worth
        // reading afterwards. Recorded on the same footing as a turn that succeeded, so a session
        // whose figures are being added up does not quietly omit the expensive failures.
        self.timing.entry(self.turns).or_default().wall_ms += self
            .finished
            .map_or(0, |f| u64::try_from(f.took.as_millis()).unwrap_or(u64::MAX));
        self.started = None;
        self.phase = None;
        self.running = None;
        self.streaming.clear();
    }

    /// Record how full the context is, against the budget it is compacted at.
    pub fn measured(&mut self, used: u64, budget: u64, guessed: bool) {
        if used == 0 {
            self.occupancy = Occupancy::Unmeasured;
        } else {
            self.occupancy = Occupancy::Measured {
                used,
                budget,
                guessed,
            };
        }
    }

    /// Record that compaction shortened the conversation underneath the session.
    pub fn compacted(&mut self) {
        self.occupancy = Occupancy::Compacted;
    }

    /// The occupancy state of the session context.
    pub fn occupancy(&self) -> Occupancy {
        self.occupancy
    }

    /// Update the context budget and guessed status against which occupancy is calculated,
    /// preserving the measured token count.
    pub fn update_budget(&mut self, budget: u64, guessed: bool) {
        if let Occupancy::Measured { used, .. } = self.occupancy {
            self.occupancy = Occupancy::Measured {
                used,
                budget,
                guessed,
            };
        }
    }

    /// Record what the last turn asked for, what the server answered with, and which tier it ran on.
    ///
    /// `comparable` says whether the two names are drawn from one roster, and so whether a difference
    /// between them means anything. The caller knows which backend answered; this cannot tell.
    pub fn served(
        &mut self,
        requested: Option<String>,
        served: impl Into<String>,
        premium: bool,
        comparable: bool,
    ) {
        self.served = Some((requested, served.into()));
        self.premium = Some(premium);
        self.served_names_are_comparable = comparable;
    }

    /// Whether the last turn spent a subscription credential, or `None` before one has run.
    pub fn premium(&self) -> Option<bool> {
        self.premium
    }

    /// The model the server last reported using, or `None` before any turn has run.
    pub fn served_model(&self) -> Option<&str> {
        self.served.as_ref().map(|(_, served)| served.as_str())
    }

    /// The model the last turn asked for, where it named one and the server answered with something
    /// else.
    ///
    /// `None` where they agree, so a caller has nothing to report on the ordinary path. Compared
    /// exactly, which works because both names come from the same roster: what was asked for is a
    /// name off a list the endpoint gave, and what answered is a name from the same list.
    ///
    /// Also `None` where the two were never comparable. A request may name an opaque handle instead
    /// of a model, and a handle standing for a different name is the indirection working rather than a
    /// substitution, so comparing them would put a warning on every turn. Whether they are comparable
    /// is recorded by whoever knew which backend answered, rather than guessed at from the spelling.
    pub fn substituted_model(&self) -> Option<&str> {
        let (requested, served) = self.served.as_ref()?;
        let requested = requested.as_deref()?;
        if !self.served_names_are_comparable {
            return None;
        }
        (requested != served).then_some(requested)
    }

    /// How full the context is, as a percentage, or `None` where nothing has been measured.
    ///
    /// Capped at a hundred rather than allowed past it. The budget is a guess at a window nobody
    /// reports, so a request larger than it is a session that will be compacted next round, not a
    /// context that is a hundred and forty per cent full.
    pub fn fullness(&self) -> Option<u64> {
        self.occupancy.percent()
    }

    /// Enter the working state for something that is not a turn.
    ///
    /// `/compact` makes a model call and takes as long as a round does, so the spinner has to run
    /// for it or the session reads as stopped at the moment it is busiest. Not a turn: nothing
    /// joins the transcript, the count of turns does not move, and the task list is left alone,
    /// since the work it describes is still outstanding afterwards.
    pub fn begin_aside(&mut self) {
        self.status = Status::Working;
        self.scroll = 0;
        self.phase = None;
        self.running = None;
        self.started = Some(Instant::now());
    }

    /// Leave it again, adding what it cost to the session's total.
    ///
    /// Charged to the turn in flight, since `/compact` is asked for in the middle of one and its
    /// cost is part of what that turn spent. Attributing it to no turn would lose it from the
    /// per-turn figures while still counting it in the total, so the two would not add up.
    pub fn end_aside(&mut self, tokens: u64) {
        self.status = Status::Idle;
        // Read before the timer is cleared. A `/compact` is a model call and nothing else, so all
        // of it is inference: charged to the turn it interrupted, exactly as its tokens are, and to
        // both figures rather than only to the wall clock, or an aside would read as time the
        // harness spent on itself.
        let took = u64::try_from(self.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.started = None;
        self.phase = None;
        self.running = None;
        self.tokens += tokens;
        if self.turns > 0 {
            *self.spend.entry(self.turns).or_insert(0) += tokens;
            let entry = self.timing.entry(self.turns).or_default();
            entry.wall_ms += took;
            entry.inference_ms += took;
        }
    }

    pub fn note(&mut self, message: impl Into<String>) {
        self.transcript.push(Entry::system(message));
    }

    /// Put a status report in the transcript, one note per line.
    ///
    /// In the transcript rather than over the screen, so it scrolls back with everything else and
    /// can be copied out with the mouse.
    ///
    /// Both columns are padded here rather than by the renderer, because this is the only place that
    /// knows the lines belong to one block. Values are aligned as well as labels: a note trailing a
    /// short value would otherwise sit wherever that value happened to end, and a column of asides
    /// starting in a different place on every row is harder to read than no column at all.
    pub fn report(&mut self, report: crate::status::Report) {
        let width_of = |pick: fn(&crate::status::Line) -> &str| {
            report
                .lines
                .iter()
                .map(|line| pick(line).chars().count())
                .max()
                .unwrap_or(0)
        };
        let labels = width_of(|line| &line.label);
        // Only rows that carry a note need their value padded, so a long value on a row without one
        // does not push every aside across the screen.
        let values = report
            .lines
            .iter()
            .filter(|line| !line.note.is_empty())
            .map(|line| line.value.chars().count())
            .max()
            .unwrap_or(0);

        for line in report.lines {
            let label = format!(
                "{}{}",
                line.label,
                " ".repeat(labels - line.label.chars().count())
            );
            if line.note.is_empty() {
                self.note(format!("{label}  {}", line.value));
                continue;
            }
            let value = format!(
                "{}{}",
                line.value,
                " ".repeat(values.saturating_sub(line.value.chars().count()))
            );
            self.note(format!("{label}  {value}  {}", line.note));
        }
    }

    /// Say something once for the whole session, ignoring it if it has been said already.
    pub fn note_once(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.said.iter().any(|said| said == &message) {
            return;
        }
        self.said.push(message.clone());
        self.note(message);
    }

    /// What the user said last time this exact question was asked, if they were asked it.
    pub fn recall_answer(&self, key: &str) -> Option<bravebot_core::ask::Answer> {
        self.answers
            .iter()
            .find(|(asked, _)| asked == key)
            .map(|(_, answer)| answer.clone())
    }

    /// Remember an answer, replacing any earlier one for the same question.
    pub fn remember_answer(&mut self, key: String, answer: bravebot_core::ask::Answer) {
        match self.answers.iter_mut().find(|(asked, _)| *asked == key) {
            Some(slot) => slot.1 = answer,
            None => self.answers.push((key, answer)),
        }
    }

    pub fn toggle_trail(&mut self) {
        self.show_trail = !self.show_trail;
    }

    /// Whether any turn has left a trail, meaning the toggle has something to reveal.
    ///
    /// A trail lands on an entry when the turn it belongs to ends, so a session that has not
    /// finished one holds nothing for the key to show.
    pub fn has_trail(&self) -> bool {
        self.transcript.iter().any(|entry| !entry.trail.is_empty())
    }

    pub fn quit(&mut self) {
        self.status = Status::Quitting;
    }

    pub fn is_quitting(&self) -> bool {
        self.status == Status::Quitting
    }

    /// Open the search over the prompt history.
    ///
    /// Seeded with whatever is in the box, when that is one line of it: somebody who typed half a
    /// prompt and then reached for the history has already said what they are looking for, and a
    /// pasted paragraph is not that. The line stays in the box, so leaving puts nothing back.
    ///
    /// Nothing to search is nothing to open. A panel over the transcript saying a person has never
    /// sent a prompt is a mode they then have to get out of.
    pub fn open_history_search(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let seed = match self.input.contains('\n') {
            true => String::new(),
            false => self.input.trim().to_string(),
        };
        self.history_search = Some(crate::history_search::Search::looking_for(seed));
        true
    }

    /// Close it, leaving the box as it was.
    pub fn close_history_search(&mut self) {
        self.history_search = None;
    }

    pub fn searching_history(&self) -> bool {
        self.history_search.is_some()
    }

    /// What is being searched for, for the renderer and for a test.
    pub fn history_search(&self) -> Option<&crate::history_search::Search> {
        self.history_search.as_ref()
    }

    /// The prompts the open search answers with, oldest first.
    ///
    /// Empty when nothing is open, so a caller need not ask twice.
    pub fn history_matches(&self) -> Vec<&crate::history::Entry> {
        match &self.history_search {
            Some(search) => search.matching(self.history.entries(), self.project().as_deref()),
            None => Vec::new(),
        }
    }

    /// The prompt under the cursor in the open search.
    pub fn history_match(&self) -> Option<&crate::history::Entry> {
        let matching = self.history_matches();
        let at = self.history_search.as_ref()?.at(matching.len())?;
        matching.get(at).copied()
    }

    /// Narrow the search by one character.
    pub fn type_into_history_search(&mut self, c: char) {
        if let Some(search) = &mut self.history_search {
            search.typed(c);
        }
    }

    /// Widen it by one, reporting whether there was anything to remove.
    pub fn backspace_history_search(&mut self) -> bool {
        match &mut self.history_search {
            Some(search) => search.backspace(),
            None => false,
        }
    }

    /// Move the cursor to an older match.
    pub fn history_search_older(&mut self) {
        let matches = self.history_matches().len();
        if let Some(search) = &mut self.history_search {
            search.older(matches);
        }
    }

    /// Move the cursor to a newer match.
    pub fn history_search_newer(&mut self) {
        if let Some(search) = &mut self.history_search {
            search.newer();
        }
    }

    /// Swap between every prompt and the ones sent from this workspace.
    pub fn scope_history_search(&mut self) {
        if let Some(search) = &mut self.history_search {
            search.scope();
        }
    }

    /// Take the prompt under the cursor into the box, and close the search.
    ///
    /// Into the box rather than sent, because a stored line is content: the file it came from can
    /// be edited, and on a shared machine by somebody else. The keystroke that sends it is the
    /// person's, exactly as it would have been had they typed the line.
    ///
    /// It replaces what was in the box, since what was in the box is what the search was seeded
    /// with: keeping both would leave the person editing their own half-typed line with the
    /// recalled one appended to it.
    pub fn take_history_match(&mut self) -> bool {
        let Some(prompt) = self.history_match().map(|entry| entry.prompt.clone()) else {
            return false;
        };
        self.close_history_search();
        self.set_input(prompt);
        self.history.leave();
        true
    }

    /// Open the scroller on the view already on the screen.
    ///
    /// Nothing about the view is touched. The scroller reads the offset the wheel writes, so the
    /// row under somebody's eye when they press the key is the row under it afterwards. The
    /// screen around it does change shape, since the box and the indicator come off it, and the
    /// rows they were using are given to the transcript. What that means for the view is settled
    /// where every other change of shape is settled, in [`Session::note_layout`]: the row at the
    /// top stays where it is, and the rows gained appear beneath it, which is where the box that
    /// gave them up was.
    pub fn open_scroller(&mut self) {
        self.scroller = Some(Scroller::default());
    }

    /// Close it, leaving the view where it was left.
    pub fn close_scroller(&mut self) {
        self.scroller = None;
    }

    pub fn scrolling(&self) -> bool {
        self.scroller.is_some()
    }

    /// What the scroller is doing, for the renderer and for a test.
    pub fn scroller(&self) -> Option<&Scroller> {
        self.scroller.as_ref()
    }

    /// Take what the last frame laid out, and hold the view on the row it was showing.
    ///
    /// The offset is counted from the end, and while the scroller is open both ends move: rows
    /// arrive underneath as a turn writes them, and the screen changes shape when the box comes
    /// off it. Either would otherwise slide the view down the transcript. Holding the view is the
    /// whole of what the scroller is for, so the offset is worked out afresh from the row that was
    /// at the top, rather than carried across a layout it was measured against.
    ///
    /// A view sitting at the tail stays at the tail, since somebody watching a reply arrive is
    /// watching the end of it, and only an open scroller holds a view against that.
    pub fn note_layout(&mut self, laid: Laid) {
        if self.scrolling() || self.scroll > 0 {
            let top = self.top_row();
            let furthest = laid.rows.saturating_sub(laid.height);
            self.scroll = furthest.saturating_sub(top);
        }
        self.laid = laid;
    }

    /// How far back the view can go before it is looking at the first row.
    fn furthest(&self) -> u16 {
        self.laid.rows.saturating_sub(self.laid.height)
    }

    /// The row at the top of the view.
    pub fn top_row(&self) -> u16 {
        self.furthest()
            .saturating_sub(self.scroll.min(self.furthest()))
    }

    /// Move the view back by `rows`, stopping at the first row rather than counting past it.
    pub fn scroller_back(&mut self, rows: u16) {
        self.scroll = self.scroll.saturating_add(rows).min(self.furthest());
    }

    /// Move the view on by `rows`, stopping at the last.
    pub fn scroller_on(&mut self, rows: u16) {
        self.scroll = self.scroll.saturating_sub(rows);
    }

    pub fn scroller_to_first_row(&mut self) {
        self.scroll = self.furthest();
    }

    pub fn scroller_to_last_row(&mut self) {
        self.scroll = 0;
    }

    /// Half a screen, and never nothing: a view one row tall still has to move when asked.
    pub fn half_screen(&self) -> u16 {
        (self.laid.height / 2).max(1)
    }

    pub fn whole_screen(&self) -> u16 {
        self.laid.height.max(1)
    }

    /// Put `row` at the top of the view.
    fn scroller_to_row(&mut self, row: u16) {
        self.scroll = self.furthest().saturating_sub(row);
    }

    /// Move to the prompt before the one the view is on, or to the first row past the earliest.
    ///
    /// Where these land is settled by what the person typed and by nothing read out of the
    /// workspace: a prompt is the one thing in a transcript they wrote themselves.
    pub fn to_previous_prompt(&mut self) {
        let top = self.top_row();
        match self.laid.prompts.iter().rev().find(|row| **row < top) {
            Some(row) => {
                let row = *row;
                self.scroller_to_row(row)
            }
            None => self.scroller_to_first_row(),
        }
    }

    /// Move to the prompt after the one the view is on, or to the last row past the latest.
    pub fn to_next_prompt(&mut self) {
        let top = self.top_row();
        match self.laid.prompts.iter().find(|row| **row > top) {
            Some(row) => {
                let row = *row;
                self.scroller_to_row(row)
            }
            None => self.scroller_to_last_row(),
        }
    }

    /// Start typing a search, with nothing in it yet.
    pub fn begin_search(&mut self) {
        if let Some(scroller) = &mut self.scroller {
            scroller.typing = Some(String::new());
        }
    }

    /// Whether a search is being typed, which is what makes a letter a letter again.
    pub fn typing_a_search(&self) -> bool {
        self.scroller
            .as_ref()
            .is_some_and(|scroller| scroller.typing.is_some())
    }

    pub fn type_into_search(&mut self, c: char) {
        if let Some(typing) = self.scroller.as_mut().and_then(|s| s.typing.as_mut()) {
            typing.push(c);
        }
    }

    /// Take the last character back, and say whether there was one.
    ///
    /// An empty needle backspaced into is the search being abandoned, which is what the key means
    /// when there is nothing left of what it deletes.
    pub fn backspace_search(&mut self) -> bool {
        match self.scroller.as_mut().and_then(|s| s.typing.as_mut()) {
            Some(typing) => typing.pop().is_some(),
            None => false,
        }
    }

    /// Clear a finished search, and say whether there was one to clear.
    ///
    /// The highlights go and the view stays. Somebody who has found what they were looking for
    /// wants the marks off the screen, not to be put back at the box.
    pub fn clear_search(&mut self) -> bool {
        match &mut self.scroller {
            Some(scroller) if !scroller.needle.is_empty() => {
                scroller.needle.clear();
                scroller.at = 0;
                true
            }
            _ => false,
        }
    }

    /// Abandon a search being typed, leaving the view where it was.
    pub fn abandon_search(&mut self) {
        if let Some(scroller) = &mut self.scroller {
            scroller.typing = None;
        }
    }

    /// Run what has been typed, and say what is now being looked for.
    ///
    /// The rows it matches are not known here. They come from a layout at the width the transcript
    /// is drawn in, which is the renderer's to do, so the caller runs this and then lands the view
    /// on what the layout found.
    pub fn run_search(&mut self) {
        if let Some(scroller) = &mut self.scroller
            && let Some(typed) = scroller.typing.take()
        {
            scroller.needle = typed;
            scroller.at = 0;
        }
    }

    /// What a finished search is looking for, which is what the renderer highlights.
    pub fn needle(&self) -> &str {
        self.scroller
            .as_ref()
            .map_or("", |scroller| scroller.needle.as_str())
    }

    /// Land on the first match at or after the top of the view, wrapping to the first of all.
    ///
    /// At or after, rather than after, because a search run while a match is already on the top
    /// row has found that one and should not step over it.
    pub fn land_on_a_match(&mut self, rows: &[u16]) {
        let top = self.top_row();
        let landing = rows
            .iter()
            .position(|row| *row >= top)
            .or(if rows.is_empty() { None } else { Some(0) });
        self.land_at(rows, landing);
    }

    /// Walk to the next match or the previous one, wrapping at either end.
    pub fn to_a_match(&mut self, rows: &[u16], forwards: bool) {
        let top = self.top_row();
        let landing = if forwards {
            rows.iter()
                .position(|row| *row > top)
                .or(if rows.is_empty() { None } else { Some(0) })
        } else {
            rows.iter()
                .rposition(|row| *row < top)
                .or(rows.len().checked_sub(1))
        };
        self.land_at(rows, landing);
    }

    fn land_at(&mut self, rows: &[u16], landing: Option<usize>) {
        let Some(index) = landing else {
            return;
        };
        let row = rows[index];
        self.scroller_to_row(row);
        if let Some(scroller) = &mut self.scroller {
            scroller.at = index;
        }
    }

    pub fn toggle_scroller_help(&mut self) {
        if let Some(scroller) = &mut self.scroller {
            scroller.help = !scroller.help;
        }
    }

    /// How many rows of transcript sit below the view, which is what has yet to be read.
    pub fn rows_below(&self) -> u16 {
        self.scroll.min(self.furthest())
    }

    pub fn scroll_up(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_add(lines);
    }

    pub fn scroll_down(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_sub(lines);
    }
}

/// The byte offset `column` characters into `line`, or its end where it is shorter.
///
/// What keeps the caret roughly where it looked while moving between lines of unequal length.
fn along(line: &str, column: usize) -> usize {
    line.char_indices()
        .nth(column)
        .map_or(line.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_core::ask::Answer;

    mod delegates {
        use super::*;
        use bravebot_agent::report::DelegateId;

        /// A delegate beginning, numbered the way the driver numbers them, and everything
        /// reported next is its work until the driver says otherwise.
        fn spawn(session: &mut Session, kind: &'static str, task: &str) -> DelegateId {
            let id = DelegateId::nth(session.delegates().len() as u32 + 1);
            session.delegate_started(bravebot_agent::report::Delegation {
                id,
                kind,
                task: task.to_string(),
            });
            session.reporting_for(Some(id));
            id
        }

        /// The key is for looking at what is happening now, and the delegate that is working is
        /// what is happening now. Anything else makes a person hunt for the one they meant.
        #[test]
        fn watching_opens_on_the_delegate_that_is_working() {
            let mut session = Session::new("none");
            let first = spawn(&mut session, "reader", "find the parser");
            session.delegate_finished(first, "answered".to_string(), false, None);
            spawn(&mut session, "checker", "run the build");

            assert!(
                session.watch(),
                "there was a delegate and the key did nothing"
            );
            assert_eq!(
                session.watched_delegate().map(|delegate| delegate.kind),
                Some("checker"),
                "the view opened on a delegate that had already finished"
            );
        }

        /// A mode that opens on an empty screen is worse than a key that does not answer: the
        /// person is now somewhere, with nothing to read and something to get out of.
        #[test]
        fn there_is_nothing_to_watch_until_a_delegate_has_run() {
            let mut session = Session::new("none");

            assert!(!session.watch(), "the view opened over no delegates at all");
            assert!(session.watching().is_none());
        }

        /// Which delegate is the question a person has when several are going, and a view that
        /// opened straight into one of them would answer a question they had not asked.
        #[test]
        fn several_delegates_are_opened_on_the_list_of_them() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            spawn(&mut session, "checker", "run the build");

            session.watch();
            assert!(
                session.listing_delegates(),
                "several delegates opened straight into one of them"
            );
            assert!(!session.watching_a_delegate());
        }

        /// One delegate is not a choice, and a list of one is a row somebody has to press through
        /// to reach the only thing behind it.
        #[test]
        fn one_delegate_is_opened_without_a_list_to_pick_from() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");

            session.watch();
            assert!(
                session.watching_a_delegate(),
                "one delegate was put behind a list of one"
            );
        }

        /// Somebody who went to look at a delegate was reading something when they left. Coming
        /// back to the end of a transcript that moved on while they were away loses their place
        /// for a reason that has nothing to do with them.
        #[test]
        fn coming_back_from_a_delegate_puts_the_turns_view_where_it_was_left() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            session.scroll = 12;

            session.watch();
            session.scroll_up(3);
            session.stop_watching();

            assert_eq!(
                session.scroll, 12,
                "the turn's view came back somewhere else"
            );
        }

        /// Reading back through a delegate must not drag the turn's own view with it, or coming
        /// back out lands somewhere nobody asked to be.
        #[test]
        fn the_turns_view_is_not_dragged_by_reading_through_a_delegate() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            session.watch();

            session.scroll_up(5);
            assert_ne!(
                session.scroll, 0,
                "reading back through a delegate moved nothing"
            );

            session.stop_watching();
            assert_eq!(
                session.scroll, 0,
                "the turn's view was dragged by the delegate's"
            );
        }

        /// Stepping through wants to arrive at the last one and know it is the last. Wrapping
        /// round to the first says the opposite, and says it silently.
        #[test]
        fn moving_between_delegates_stops_at_each_end() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            spawn(&mut session, "checker", "run the build");
            session.watch();
            session.open_watched();
            session.watch_previous();

            session.watch_previous();
            assert_eq!(session.watching().map(|watching| watching.at), Some(0));

            session.watch_next();
            session.watch_next();
            assert_eq!(
                session.watching().map(|watching| watching.at),
                Some(1),
                "moving past the last delegate wrapped round to the first"
            );
        }

        /// The way back was the one destination the list did not offer: somebody comparing two
        /// delegates could reach either and could not reach what they were reading before.
        #[test]
        fn moving_up_from_the_first_delegate_in_the_list_reaches_the_session() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            spawn(&mut session, "checker", "run the build");
            session.watch();
            session.watch_previous();

            assert!(
                !session.listing_on_the_session(),
                "the highlight left the delegates before reaching the first of them"
            );

            session.watch_previous();
            assert!(
                session.listing_on_the_session(),
                "moving up from the first delegate did not reach the session"
            );
            assert_eq!(
                session.list_highlight(),
                0,
                "the session is not the first row of the list"
            );
            assert!(
                session.watched().is_none(),
                "the session row was reported as a delegate"
            );
        }

        /// What n and p are for in a delegate's own view is comparing two runs. A key that
        /// stepped out of the mode partway through would be a different key wearing the name.
        #[test]
        fn the_session_is_not_a_step_in_a_delegates_own_view() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            spawn(&mut session, "checker", "run the build");
            session.watch();
            session.open_watched();
            session.watch_previous();

            session.watch_previous();
            assert!(
                !session.listing_on_the_session(),
                "stepping back past the first delegate left the delegates"
            );
            assert_eq!(
                session.watched_delegate().map(|delegate| delegate.kind),
                Some("reader"),
                "the view stopped being on a delegate"
            );
        }

        /// Going back to the list from a delegate puts the highlight on that delegate. Landing on
        /// the session instead would offer the way out to somebody who asked for the way back.
        #[test]
        fn going_back_to_the_list_lands_on_the_delegate_that_was_open() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            spawn(&mut session, "checker", "run the build");
            session.watch();
            session.open_watched();

            assert!(session.list_delegates());
            assert!(
                !session.listing_on_the_session(),
                "coming back from a delegate landed on the session"
            );
            assert_eq!(
                session.watched_delegate().map(|delegate| delegate.kind),
                Some("checker")
            );
        }

        /// What is on the screen changes when a person asks and not otherwise. A delegate that
        /// answers while somebody is reading it has not asked for anything.
        #[test]
        fn a_delegate_that_finishes_is_still_the_one_being_watched() {
            let mut session = Session::new("none");
            let id = spawn(&mut session, "reader", "find the parser");
            session.watch();

            session.delegate_finished(id, "found it in state.rs".to_string(), false, None);
            assert_eq!(
                session.watched_delegate().map(|delegate| delegate.kind),
                Some("reader"),
                "a delegate finishing took the screen away from it"
            );
        }

        /// The same rule from the other side: a turn that spawns a fourth delegate while somebody
        /// is reading the second must not move them to the fourth.
        #[test]
        fn a_new_delegate_does_not_take_the_screen_from_the_one_being_read() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            spawn(&mut session, "checker", "run the build");
            session.watch();
            session.open_watched();
            session.watch_previous();

            spawn(&mut session, "worker", "write it down");
            assert_eq!(
                session.watched_delegate().map(|delegate| delegate.kind),
                Some("reader"),
                "a delegate starting took the screen from the one being read"
            );
        }

        /// The whole of what the mode is for: the lines on the screen are the delegate's own.
        #[test]
        fn watching_a_delegate_shows_its_lines_rather_than_the_turns() {
            let mut session = Session::new("none");
            session.reporting_for(None);
            session
                .transcript
                .push(Entry::user("what is in the parser"));
            spawn(&mut session, "reader", "find the parser");
            session.start_activity(Activity::running("Read", "parser.rs"));

            session.watch();
            let viewed = session.viewed();
            assert_eq!(
                viewed.len(),
                1,
                "the turn's own lines were in the delegate's view"
            );
            assert_eq!(
                viewed[0].activity.as_ref().unwrap().target,
                "parser.rs",
                "the delegate's own line was not what was viewed"
            );
        }

        /// A delegate belongs to the conversation that spawned it, and the mode standing over one
        /// after the conversation has gone is standing over nothing.
        #[test]
        fn clearing_closes_the_view_over_a_delegate() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            session.watch();

            session.clear();
            assert!(
                session.watching().is_none(),
                "the view outlived its delegates"
            );
            assert!(session.viewed().is_empty());
        }

        fn ran(session: &mut Session, command: &str, read: bool) {
            session.command_printed(bravebot_agent::report::Printed {
                command: command.to_string(),
                lines: vec!["first".to_string(), "second".to_string()],
                total: 2,
                read_by_the_planner: read,
                outcome: bravebot_agent::report::Outcome::Succeeded,
            });
        }

        /// A command's output is drawn nowhere else in full, which is the same reason a delegate's
        /// work has a view: a line saying "12 lines, quarantined" does not tell somebody who owns
        /// the directory what their agent just ran.
        #[test]
        fn a_command_this_session_ran_is_something_the_view_can_open() {
            let mut session = Session::new("none");
            assert!(!session.watch(), "the key opened on nothing");

            ran(&mut session, "cargo test", false);
            assert!(session.watch(), "a command was not something to look at");
            assert!(matches!(session.watched(), Some(Watched::Output(_))));
            assert_eq!(
                session.watched_output().map(|o| o.command.as_str()),
                Some("cargo test")
            );
        }

        /// One list rather than two. Which kind of work a row is is a property of the row, not a
        /// reason for a second key to learn.
        #[test]
        fn the_list_holds_delegates_and_commands_together() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            ran(&mut session, "cargo test", true);

            let rows = session.watchable();
            assert_eq!(rows.len(), 2);
            assert!(matches!(rows[0], Watched::Delegate(_)));
            assert!(matches!(rows[1], Watched::Output(_)));
        }

        /// Delegates first and commands after them, so a row's place does not move under somebody
        /// stepping through it when the next command runs.
        #[test]
        fn a_rows_place_in_the_list_does_not_move_when_the_next_command_runs() {
            let mut session = Session::new("none");
            ran(&mut session, "first", true);
            session.watch();
            let before = session.watched_output().map(|o| o.command.clone());
            ran(&mut session, "second", true);
            assert_eq!(
                session.watched_output().map(|o| o.command.clone()),
                before,
                "the view moved to a command nobody asked for"
            );
        }

        /// The one thing a person cannot work out from the bytes, and the thing the whole design
        /// turns on: the same output either reached a model's context or did not.
        #[test]
        fn a_command_row_keeps_whether_the_planner_read_it() {
            let mut session = Session::new("none");
            ran(&mut session, "cat secret", false);
            ran(&mut session, "cargo test", true);
            let kept: Vec<bool> = session
                .outputs()
                .iter()
                .map(|output| output.read_by_the_planner)
                .collect();
            assert_eq!(kept, [false, true]);
        }

        /// Stepping through the list reaches both kinds, since it is one list and the keys that
        /// move in it do not ask what a row is.
        #[test]
        fn stepping_through_the_list_reaches_a_command_after_a_delegate() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            ran(&mut session, "cargo test", true);

            session.watch();
            assert!(
                session.listing_delegates(),
                "two rows did not open the list"
            );
            while !matches!(session.watched(), Some(Watched::Output(_))) {
                let before = session.list_highlight();
                session.watch_next();
                assert_ne!(before, session.list_highlight(), "the list stopped moving");
            }
            session.open_watched();
            assert!(session.watched_output().is_some());
        }

        /// The point of delegating is that the reading lands somewhere else, and the interface
        /// has the same problem the planner does: a turn that asked a delegate to run the build
        /// should not have the build log in the middle of it.
        #[test]
        fn a_delegates_work_goes_under_its_own_block_and_not_into_the_turns_lines() {
            let mut session = Session::new("none");
            session.start_activity(Activity::running("Delegate", ""));
            spawn(&mut session, "reader", "find the parser");
            session.start_activity(Activity::running("Read", "src/parse.rs"));

            let drawn: Vec<&str> = session
                .transcript
                .iter()
                .map(|entry| entry.text.as_str())
                .collect();
            assert!(
                !drawn.iter().any(|text| text.contains("parse.rs")),
                "a delegate's own call was drawn as the turn's: {drawn:?}"
            );
            assert_eq!(
                session.delegates()[0].lines.len(),
                1,
                "the delegate's call was not kept under it"
            );
        }

        /// Two delegates work at once and their reports interleave, so the only thing saying
        /// whose a line is is what the driver said. Two of the same kind produce lines that read
        /// identically.
        #[test]
        fn each_delegates_work_lands_under_the_delegate_that_did_it() {
            let mut session = Session::new("none");
            let first = spawn(&mut session, "reader", "read a.txt");
            let second = spawn(&mut session, "reader", "read b.txt");

            // Interleaved the way they arrive: the second one's call, then the first one's.
            session.reporting_for(Some(second));
            session.start_activity(Activity::running("Read", "b.txt"));
            session.reporting_for(Some(first));
            session.start_activity(Activity::running("Read", "a.txt"));

            let held = session.delegates();
            assert_eq!(held[0].lines[0].activity.as_ref().unwrap().target, "a.txt");
            assert_eq!(held[1].lines[0].activity.as_ref().unwrap().target, "b.txt");
        }

        /// A block that stopped without saying how leaves somebody looking at a last tool call,
        /// unable to tell an answer from a failure.
        #[test]
        fn what_a_delegate_ended_with_closes_its_block() {
            let mut session = Session::new("none");
            let id = spawn(&mut session, "reader", "find the parser");
            session.start_activity(Activity::running("Read", "src/parse.rs"));
            session.delegate_finished(
                id,
                "a reader delegate answered after 2 rounds".to_string(),
                false,
                None,
            );

            let held = session.delegates();
            assert!(!held[0].is_running());
            assert_eq!(
                held[0].note.as_deref(),
                Some("a reader delegate answered after 2 rounds")
            );
        }

        /// The turn's own lines come back once the driver says the delegate is done, or every
        /// later line would land under a delegate that stopped working some time ago.
        #[test]
        fn the_turns_own_lines_come_back_once_a_delegate_has_finished() {
            let mut session = Session::new("none");
            let id = spawn(&mut session, "reader", "find the parser");
            session.reporting_for(None);
            session.delegate_finished(id, "answered".to_string(), false, None);
            session.start_activity(Activity::running("Read", "afterwards.rs"));

            assert!(
                session.delegates()[0].lines.is_empty(),
                "a line reported after the delegate finished landed under it"
            );
            assert!(
                session.transcript.iter().any(|entry| entry
                    .activity
                    .as_ref()
                    .is_some_and(|a| a.target == "afterwards.rs")),
                "the turn's own line did not come back"
            );
        }

        /// There is one model writing at a time. Drawn in the turn's own view, a delegate's
        /// half-written sentence reads as the planner writing something it never wrote.
        #[test]
        fn a_reply_a_delegate_is_writing_is_not_drawn_over_the_turn() {
            let mut session = Session::new("none");
            session.streaming("the turn is thinking");
            spawn(&mut session, "reader", "find the parser");
            session.streaming(" and now the delegate is");

            assert_eq!(session.streaming, "the turn is thinking");
        }

        /// The block where a delegate started is a glance at it rather than the whole of it. It
        /// draws the newest few and says how much has happened, so three rows under a delegate
        /// that has made thirty calls do not read as a delegate doing very little.
        #[test]
        fn a_delegates_block_draws_the_last_of_its_work_and_counts_the_rest() {
            let mut session = Session::new("none");
            spawn(&mut session, "checker", "run everything");
            for round in 0..8 {
                session.start_activity(Activity::running("Run", format!("step {round}")));
            }

            let held = session.delegates();
            assert_eq!(held[0].calls, 8, "the block forgot how much it had done");
            assert_eq!(
                held[0].latest().len(),
                DELEGATE_SHOWN,
                "the block drew more than it says it draws"
            );
            assert_eq!(
                held[0]
                    .latest()
                    .last()
                    .unwrap()
                    .activity
                    .as_ref()
                    .unwrap()
                    .target,
                "step 7",
                "the block drew the oldest lines rather than the newest"
            );
        }

        /// What the block draws is a window on what is kept. Keeping only the three drawn is what
        /// left the mode that opens over a delegate with three rows to show for an hour's work.
        #[test]
        fn a_delegate_keeps_the_work_its_block_has_no_room_for() {
            let mut session = Session::new("none");
            spawn(&mut session, "checker", "run everything");
            for round in 0..8 {
                session.start_activity(Activity::running("Run", format!("step {round}")));
            }

            assert_eq!(
                session.delegates()[0].lines.len(),
                8,
                "a delegate threw away the work its block had no room for"
            );
        }

        /// A delegate can run for as long as a turn does, and this is held in memory for a person
        /// who may never look at it.
        #[test]
        fn a_delegate_stops_keeping_its_oldest_work() {
            let mut session = Session::new("none");
            spawn(&mut session, "worker", "the long one");
            for round in 0..(DELEGATE_KEPT + 50) {
                session.start_activity(Activity::running("Run", format!("step {round}")));
            }

            let held = session.delegates();
            assert_eq!(
                held[0].calls,
                DELEGATE_KEPT + 50,
                "the count was capped too"
            );
            assert_eq!(held[0].lines.len(), DELEGATE_KEPT, "nothing was dropped");
            assert_eq!(
                held[0].lines[0].activity.as_ref().unwrap().target,
                "step 50",
                "the newest were dropped rather than the oldest"
            );
        }

        /// A delegate belongs to the conversation that started it. Nothing crossed back from one
        /// but the report, and the report went into a turn the new conversation does not have.
        #[test]
        fn clearing_forgets_the_delegates() {
            let mut session = Session::new("none");
            spawn(&mut session, "reader", "find the parser");
            ran(&mut session, "cargo test", false);
            session.clear();

            assert!(session.delegates().is_empty());
            assert!(
                session.watchable().is_empty(),
                "a command from the forgotten conversation is still openable"
            );
        }
    }

    /// The endpoint substitutes rather than refusing, so a session that asked for one model and was
    /// answered by another has to be told: nothing else in the reply says so.
    #[test]
    fn a_model_answered_by_a_different_one_is_reported_as_substituted() {
        let mut session = Session::new("none");
        session.choose_model("claude-opus".to_string());
        session.served(Some("claude-opus".to_string()), "qwen-14b", false, true);
        assert_eq!(session.substituted_model(), Some("claude-opus"));
    }

    /// A request may name a handle standing for a model rather than a model, and then the reply names
    /// something different every single time. Reported, that is a warning on every turn about nothing
    /// being wrong, which is how a real substitution stops being noticeable.
    #[test]
    fn a_request_whose_name_was_never_comparable_reports_no_substitution() {
        let mut session = Session::new("none");
        session.choose_model(
            "arn:aws:bedrock:us-west-2:1:application-inference-profile/x".to_string(),
        );
        session.served(
            Some("arn:aws:bedrock:us-west-2:1:application-inference-profile/x".to_string()),
            "claude-sonnet-5",
            false,
            false,
        );
        assert_eq!(session.substituted_model(), None);
    }

    /// The ordinary path: asked for and answered by the same model, so there is nothing to say.
    #[test]
    fn a_model_answered_by_itself_is_not_a_substitution() {
        let mut session = Session::new("none");
        session.choose_model("claude-opus".to_string());
        session.served(Some("claude-opus".to_string()), "claude-opus", false, true);
        assert_eq!(session.substituted_model(), None);
    }

    /// A needle is a run of characters, and finding it is finding those characters. Anything
    /// cleverer is a pattern language, and a pattern language here would be an interpreter
    /// reached by a line typed over text somebody else may have written.
    #[test]
    fn a_search_matches_a_substring_literally() {
        assert_eq!(matched("hello world", "lo wo"), vec![(3, 8)]);
        assert_eq!(matched("aaaa", "aa"), vec![(0, 2), (2, 4)]);
        assert_eq!(matched("hello", "goodbye"), Vec::new());
        assert_eq!(matched("hello", ""), Vec::new());
    }

    /// The rule every search box already uses, so nobody has to be told it: type in lower case
    /// and you are not asking about case, type a capital and you are.
    #[test]
    fn a_needle_in_lower_case_matches_either_case() {
        assert_eq!(matched("Hello There", "hello"), vec![(0, 5)]);
        assert_eq!(matched("SHOUTING", "shouting"), vec![(0, 8)]);
    }

    #[test]
    fn a_needle_holding_a_capital_matches_exactly() {
        assert_eq!(matched("hello", "Hello"), Vec::new());
        assert_eq!(matched("Hello hello", "Hello"), vec![(0, 5)]);
    }

    /// The characters somebody typed, and not what a regular expression would have made of them.
    /// A dot is a dot and a star is a star, which is also what makes the scan unable to backtrack.
    #[test]
    fn a_pattern_is_matched_as_the_characters_it_is_spelled_with() {
        assert_eq!(matched("abc", "a.c"), Vec::new());
        assert_eq!(matched("a.c", "a.c"), vec![(0, 3)]);
        assert_eq!(matched("anything at all", ".*"), Vec::new());
        assert_eq!(matched("one [two] three", "[two]"), vec![(4, 9)]);
    }

    /// Walking the matches has to reach every one of them, and reach them again: somebody who
    /// passes the one they wanted presses the key once more rather than starting the search over.
    #[test]
    fn n_and_shift_n_walk_the_matches_and_wrap() {
        let mut session = Session::new("kernel-enforced");
        session.open_scroller();
        session.note_layout(Laid {
            width: 80,
            height: 10,
            rows: 100,
            ..Laid::default()
        });
        let rows = [20u16, 50, 80];

        session.scroller_to_first_row();
        session.to_a_match(&rows, true);
        assert_eq!(session.top_row(), 20);
        session.to_a_match(&rows, true);
        assert_eq!(session.top_row(), 50);
        session.to_a_match(&rows, true);
        assert_eq!(session.top_row(), 80);
        session.to_a_match(&rows, true);
        assert_eq!(session.top_row(), 20, "the walk did not wrap round");

        session.to_a_match(&rows, false);
        assert_eq!(session.top_row(), 80, "walking back did not wrap round");
        session.to_a_match(&rows, false);
        assert_eq!(session.top_row(), 50);
    }

    /// A search run while a match is already at the top of the view has found that one, and
    /// stepping over it would mean the first press of the key skipped the answer.
    #[test]
    fn a_search_lands_on_the_match_it_is_already_looking_at() {
        let mut session = Session::new("kernel-enforced");
        session.open_scroller();
        session.note_layout(Laid {
            width: 80,
            height: 10,
            rows: 100,
            ..Laid::default()
        });

        session.land_on_a_match(&[90]);
        assert_eq!(session.top_row(), 90);
        session.land_on_a_match(&[90]);
        assert_eq!(session.top_row(), 90);
    }

    /// Opening the scroller takes the box off the screen and gives the transcript its rows, and
    /// closing it hands them back. Neither is a reason for what somebody is reading to move: the
    /// rows gained appear where the box was, which is beneath what is already on the screen, and
    /// the rows given back are covered by the box coming home.
    #[test]
    fn the_row_at_the_top_of_the_view_survives_the_screen_changing_shape() {
        let mut session = Session::new("kernel-enforced");
        session.note_layout(Laid {
            width: 80,
            height: 18,
            rows: 100,
            ..Laid::default()
        });
        session.scroll_up(20);
        let looking_at = session.top_row();

        session.open_scroller();
        session.note_layout(Laid {
            width: 80,
            height: 23,
            rows: 100,
            ..Laid::default()
        });
        assert_eq!(
            session.top_row(),
            looking_at,
            "the box coming off the screen took the view with it"
        );

        session.close_scroller();
        session.note_layout(Laid {
            width: 80,
            height: 18,
            rows: 100,
            ..Laid::default()
        });
        assert_eq!(
            session.top_row(),
            looking_at,
            "the box coming back took the view with it"
        );
    }

    /// Holding the view is the whole of what the scroller is for. The offset is counted from the
    /// end and the end keeps moving, so rows arriving underneath would otherwise slide the view
    /// down the transcript while somebody was reading it.
    #[test]
    fn what_arrives_while_the_scroller_is_open_does_not_move_the_view() {
        let mut session = Session::new("kernel-enforced");
        session.note_layout(Laid {
            width: 80,
            height: 10,
            rows: 100,
            ..Laid::default()
        });
        session.open_scroller();
        session.scroller_back(40);
        let looking_at = session.top_row();

        session.note_layout(Laid {
            width: 80,
            height: 10,
            rows: 130,
            ..Laid::default()
        });

        assert_eq!(
            session.top_row(),
            looking_at,
            "thirty rows arrived and took the view with them"
        );
    }

    /// At rest the transcript follows what is being written, which is what somebody watching a
    /// reply arrive is watching it for. Only the scroller holds a view against the tail.
    #[test]
    fn what_arrives_at_rest_still_reaches_the_bottom_of_the_screen() {
        let mut session = Session::new("kernel-enforced");
        session.note_layout(Laid {
            width: 80,
            height: 10,
            rows: 100,
            ..Laid::default()
        });
        session.note_layout(Laid {
            width: 80,
            height: 10,
            rows: 130,
            ..Laid::default()
        });

        assert_eq!(session.scroll, 0, "the view was held back from the tail");
    }

    /// The end of the transcript is wherever it is now, not where it was when the scroller
    /// opened: what arrived underneath is the thing somebody pressing this key wants to see.
    #[test]
    fn the_last_row_reached_from_the_scroller_includes_what_arrived() {
        let mut session = Session::new("kernel-enforced");
        session.note_layout(Laid {
            width: 80,
            height: 10,
            rows: 100,
            ..Laid::default()
        });
        session.open_scroller();
        session.scroller_back(40);
        session.note_layout(Laid {
            width: 80,
            height: 10,
            rows: 130,
            ..Laid::default()
        });

        session.scroller_to_last_row();
        assert_eq!(session.top_row(), 120, "the view stopped at the old end");
    }

    /// A question nobody has been asked has no answer to recall, which is what makes the memo
    /// safe to consult for every question in a series.
    #[test]
    fn a_question_never_asked_has_no_remembered_answer() {
        let session = Session::new(".");
        assert_eq!(session.recall_answer("pick one: Cache: Which?"), None);
    }

    /// The key is the whole question, so two that differ anywhere are two questions and the
    /// second is put to the person rather than answered with the first one's reply.
    #[test]
    fn a_different_question_is_not_answered_from_an_earlier_one() {
        let mut session = Session::new(".");
        session.remember_answer("pick one: Cache: Which?".into(), Answer::Chosen(vec![0]));
        assert_eq!(session.recall_answer("pick one: Branch: Which?"), None);
    }

    /// Answering again replaces rather than accumulates, or the memo would grow a second entry
    /// for the same question and recall would keep returning the stale one.
    #[test]
    fn answering_the_same_question_again_replaces_what_was_remembered() {
        let mut session = Session::new(".");
        session.remember_answer("q".into(), Answer::Chosen(vec![0]));
        session.remember_answer("q".into(), Answer::Chosen(vec![1]));
        assert_eq!(session.answers.len(), 1);
        assert_eq!(session.recall_answer("q"), Some(Answer::Chosen(vec![1])));
    }

    /// A decline is an answer, so it is remembered as one. Treating it as absence would put a
    /// question the person deliberately passed over back in front of them.
    #[test]
    fn a_skipped_question_is_remembered_as_skipped() {
        let mut session = Session::new(".");
        session.remember_answer("q".into(), Answer::Declined);
        assert_eq!(session.recall_answer("q"), Some(Answer::Declined));
    }
    use bravebot_core::label::Label;

    fn session() -> Session {
        Session::new("kernel-enforced")
    }

    /// Skills and standing instructions are looked for afresh every turn, so the reason one was
    /// left out recurs every turn too. Repeating it would bury the work in a condition the user
    /// already knows about and cannot fix from here.
    #[test]
    fn a_note_said_once_is_not_said_again() {
        let mut s = session();
        s.note_once("AGENTS.md was not loaded: this directory is not trusted");
        s.note_once("AGENTS.md was not loaded: this directory is not trusted");
        s.note_once("AGENTS.md was not loaded: this directory is not trusted");

        assert_eq!(s.transcript.len(), 1, "the same note was repeated");
    }

    /// Once per message, not once ever. A second condition still needs saying.
    #[test]
    fn a_different_note_is_still_said() {
        let mut s = session();
        s.note_once("one thing happened");
        s.note_once("another thing happened");

        assert_eq!(s.transcript.len(), 2);
    }

    #[test]
    fn typing_accumulates_input() {
        let mut s = session();
        s.type_char('h');
        s.type_char('i');
        assert_eq!(s.input, "hi");
        s.backspace();
        assert_eq!(s.input, "h");
    }

    fn picture(bytes: &[u8]) -> crate::clipboard::Image {
        crate::clipboard::Image {
            media_type: "image/png",
            bytes: bytes.to_vec(),
        }
    }

    /// A picture has to leave a mark on the line, or the prompt says nothing about what is going
    /// with it and a user is left counting their own pastes to work out what the planner will see.
    #[test]
    fn a_pasted_picture_writes_a_marker_where_the_caret_is() {
        let mut s = session();
        for c in "look at ".chars() {
            s.type_char(c);
        }
        s.attach(picture(b"pixels"));
        for c in " please".chars() {
            s.type_char(c);
        }

        assert_eq!(s.input, "look at [Image #1] please");
    }

    /// The marker is the handle, so deleting it is how a picture is taken back. Without that a
    /// paste would be final, and the only way out of one would be clearing the whole line.
    #[test]
    fn deleting_a_marker_takes_the_picture_back() {
        let mut s = session();
        s.attach(picture(b"pixels"));
        for _ in 0.."[Image #1]".len() {
            s.backspace();
        }
        for c in "never mind".chars() {
            s.type_char(c);
        }

        let sent = s.submit().expect("submitted");
        assert_eq!(sent, "never mind");
        assert!(
            s.sent_pasted().is_empty(),
            "a picture nothing referred to was sent"
        );
    }

    /// A marker is one thing on the screen, so it is one press to get rid of. Nibbling a character
    /// off the end would leave text that still reads as an attachment behind a picture that is no
    /// longer attached, and the user would only find out by carrying on pressing.
    #[test]
    fn one_backspace_takes_the_whole_marker() {
        let mut s = session();
        for c in "look at ".chars() {
            s.type_char(c);
        }
        s.attach(picture(b"pixels"));
        s.backspace();

        assert_eq!(s.input, "look at ");
        assert!(
            s.pasted_named(&s.input).is_empty(),
            "the picture outlived its marker"
        );
    }

    /// One press of the arrow key crosses a marker, in either direction. It stands for one thing
    /// and reads as one thing, so counting the characters it happens to be spelled with is a
    /// dozen presses to cross what looks like a single word.
    #[test]
    fn the_caret_steps_over_a_marker_whole() {
        let mut s = session();
        for c in "look at ".chars() {
            s.type_char(c);
        }
        s.attach(picture(b"pixels"));

        s.move_left();
        assert_eq!(s.caret(), "look at ".len());

        s.move_right();
        assert_eq!(s.caret(), "look at [Image #1]".len());
    }

    /// The property behind stepping over one whole: there is nowhere inside a marker for the
    /// caret to be. A caret between two halves of a picture is a caret in a place the user cannot
    /// see, and the next thing they type would land there.
    #[test]
    fn the_caret_cannot_come_to_rest_inside_a_marker() {
        let mut s = session();
        s.attach(picture(b"pixels"));
        for c in " please".chars() {
            s.type_char(c);
        }

        let inside = 1.."[Image #1]".len();
        for _ in 0..s.input.len() {
            s.move_left();
            assert!(
                !inside.contains(&s.caret()),
                "the caret rested inside a marker"
            );
        }
        for _ in 0..s.input.len() {
            s.move_right();
            assert!(
                !inside.contains(&s.caret()),
                "the caret rested inside a marker"
            );
        }
    }

    /// Delete takes what the caret is on, and the caret is on the whole marker: the half of the
    /// line Backspace cannot reach must not be the half where a marker can be broken.
    #[test]
    fn delete_forward_takes_the_whole_marker() {
        let mut s = session();
        s.attach(picture(b"pixels"));
        for c in " please".chars() {
            s.type_char(c);
        }
        // Back over the words, and then the one press that crosses the marker.
        for _ in 0.." please".len() + 1 {
            s.move_left();
        }
        assert_eq!(s.caret(), 0);
        s.delete_forward();

        assert_eq!(s.input, " please");
        assert!(
            s.pasted_named(&s.input).is_empty(),
            "the picture outlived its marker"
        );
    }

    /// The caret covers a marker whole, so a press on it takes what is covered. Taking the
    /// character in front instead deletes something the user can see is not the thing selected.
    #[test]
    fn backspace_on_a_covered_marker_takes_the_marker() {
        let mut s = session();
        for c in "abc".chars() {
            s.type_char(c);
        }
        s.attach(picture(b"pixels"));
        for c in "xyz".chars() {
            s.type_char(c);
        }
        // Back onto the marker, which the caret then covers.
        for _ in 0.."xyz".len() + 1 {
            s.move_left();
        }
        s.backspace();

        assert_eq!(s.input, "abcxyz");
        assert!(
            s.pasted_named(&s.input).is_empty(),
            "the picture outlived its marker"
        );
    }

    /// A marker for folded words goes whole for the same reason a picture's does, and taking it
    /// takes the words behind it rather than leaving them to arrive unannounced.
    #[test]
    fn one_backspace_takes_the_whole_folded_paste() {
        let mut s = session();
        for c in "look: ".chars() {
            s.type_char(c);
        }
        s.paste_text("one\ntwo\nthree\nfour");
        s.backspace();

        assert_eq!(s.input, "look: ");
        assert_eq!(s.unfolded(s.input()), "look: ");
    }

    /// Only a marker goes whole. Ordinary square brackets are something the user typed and are
    /// deleted a character at a time, like every other character they typed.
    #[test]
    fn text_that_merely_looks_like_a_marker_is_deleted_one_character_at_a_time() {
        let mut s = session();
        for c in "[Image #7]".chars() {
            s.type_char(c);
        }
        s.backspace();

        assert_eq!(s.input, "[Image #7");
    }

    /// The pictures travel with the words they were pasted into, in the order the markers number
    /// them, since a model reading "[Image #2]" has to be able to count to the one that answers it.
    #[test]
    fn a_submitted_prompt_carries_the_pictures_it_still_refers_to() {
        let mut s = session();
        s.attach(picture(b"first"));
        s.attach(picture(b"second"));

        let sent = s.submit().expect("submitted");
        assert_eq!(sent, "[Image #1][Image #2]");
        assert_eq!(
            s.sent_pasted()
                .iter()
                .map(|i| i.bytes.clone())
                .collect::<Vec<_>>(),
            vec![b"first".to_vec(), b"second".to_vec()]
        );
    }

    /// A number is never used twice, even after the marker holding it is deleted. Reusing one
    /// would renumber the marker sitting in the line the user is looking at, and the picture
    /// behind it would quietly become a different picture.
    #[test]
    fn a_deleted_marker_does_not_free_its_number() {
        let mut s = session();
        s.attach(picture(b"first"));
        for _ in 0.."[Image #1]".len() {
            s.backspace();
        }
        s.attach(picture(b"second"));

        assert_eq!(s.input, "[Image #2]");
        s.submit().expect("submitted");
        assert_eq!(
            s.sent_pasted().len(),
            1,
            "the deleted picture was still attached"
        );
        assert_eq!(s.sent_pasted()[0].bytes, b"second".to_vec());
    }

    /// Recalling an older prompt replaces the line and every marker in it, so the pictures that
    /// belonged to the line that went away must not follow the line that came back.
    #[test]
    fn a_recalled_prompt_does_not_bring_another_prompts_pictures() {
        let mut s = session();
        s.attach(picture(b"pixels"));
        s.set_input("something else entirely");

        s.submit().expect("submitted");
        assert!(
            s.sent_pasted().is_empty(),
            "a picture followed a line it was never pasted into"
        );
    }

    /// Cancelling puts the prompt back for editing, and a prompt that came back without its
    /// pictures would come back with markers naming nothing, which the user could not see.
    #[test]
    fn a_cancelled_turn_gives_the_pictures_back_with_the_words() {
        let mut s = session();
        for c in "look at ".chars() {
            s.type_char(c);
        }
        s.attach(picture(b"pixels"));
        let sent = s.submit().expect("submitted");

        s.restore(sent);

        assert_eq!(s.input, "look at [Image #1]");
        s.submit().expect("submitted");
        assert_eq!(s.sent_pasted().len(), 1, "the picture did not come back");
    }

    /// A pasted paragraph keeps its lines: it was written with them, and the box draws them.
    /// Line endings from anywhere land as the same thing, so text copied out of a document
    /// written on Windows does not arrive with the returns still in it.
    #[test]
    fn a_paste_keeps_its_lines_however_they_were_written() {
        let mut s = session();
        s.paste("first\r\nsecond\rthird\nfourth");
        assert_eq!(s.input, "first\nsecond\nthird\nfourth");
    }

    /// Three lines with nothing after the last of them read fine in the box, and the box is where
    /// a user reads back what they are about to send. Folding there would put words somebody can
    /// still see out of their reach for nothing.
    #[test]
    fn a_short_paste_lands_in_the_box_whole() {
        let mut s = session();
        s.paste_text("first\nsecond\nthird");
        assert_eq!(s.input, "first\nsecond\nthird");
    }

    /// The third newline is where a paste starts taking the screen from the conversation it is
    /// about, so that is where it is put away.
    #[test]
    fn the_third_newline_folds_a_paste_behind_a_marker() {
        let mut s = session();
        s.paste_text("first\nsecond\nthird\n");
        assert_eq!(s.input, "[Pasted text #1 +3 lines]");
    }

    /// A trailing newline ends the last line rather than starting an empty one. A count claiming a
    /// line nobody can see is a count nobody can check.
    #[test]
    fn a_folded_paste_counts_the_lines_a_person_would_count() {
        let mut s = session();
        s.paste_text("first\nsecond\nthird\nfourth");
        assert_eq!(s.input, "[Pasted text #1 +4 lines]");
    }

    /// Line endings are settled before the lines are counted, so text copied out of a document
    /// written on Windows folds at the same place as the same text copied from anywhere else.
    #[test]
    fn a_paste_folds_the_same_however_its_lines_were_written() {
        let mut s = session();
        s.paste_text("first\r\nsecond\r\nthird\r\n");
        assert_eq!(s.input, "[Pasted text #1 +3 lines]");
    }

    /// The marker goes where the caret is and the rest of the line is left alone, because the
    /// paste belongs to the sentence it was pasted into.
    #[test]
    fn a_folded_paste_leaves_the_words_around_it_alone() {
        let mut s = session();
        for c in "what is ".chars() {
            s.type_char(c);
        }
        s.paste_text("one\ntwo\nthree\n");
        for c in " about".chars() {
            s.type_char(c);
        }
        assert_eq!(s.input, "what is [Pasted text #1 +3 lines] about");
    }

    /// Folding is a way of drawing a long line, not a way of sending one: the planner is given
    /// what pasting into the box has always given it.
    #[test]
    fn a_folded_paste_is_put_back_where_the_line_leaves_the_box() {
        let mut s = session();
        for c in "what is ".chars() {
            s.type_char(c);
        }
        s.paste_text("one\ntwo\nthree\n");

        let prompt = s.submit().expect("submitted");
        assert_eq!(prompt, "what is one\ntwo\nthree\n");
    }

    /// The scrollback is the record of what was said, and what was said is the paste. A marker
    /// left in it has the conversation claim something the planner was never given.
    #[test]
    fn the_transcript_shows_the_words_a_folded_paste_stood_for() {
        let mut s = session();
        for c in "look at ".chars() {
            s.type_char(c);
        }
        s.paste_text("one\ntwo\nthree\n");
        s.submit().expect("submitted");

        let entry = s
            .transcript
            .last()
            .expect("the prompt is in the transcript");
        assert_eq!(entry.speaker, Speaker::User);
        assert_eq!(entry.text, "look at one\ntwo\nthree\n");
    }

    /// A marker is a handle on text only the session holding it can put back. Remembered as one,
    /// a prompt comes back in a later session naming nothing, and the placeholder is sent in
    /// place of everything the person pasted with nothing on the screen to say so.
    #[test]
    fn a_folded_paste_is_remembered_as_the_words_it_stood_for() {
        let mut s = session();
        s.paste_text("one\ntwo\nthree\n");
        s.submit().expect("submitted");

        let entry = s.history.entries().last().expect("an entry");
        assert_eq!(entry.prompt, "one\ntwo\nthree\n");
    }

    /// A prompt sent while a turn runs has been sent, so it is remembered as the words too.
    #[test]
    fn a_paste_queued_behind_a_turn_is_remembered_as_its_words() {
        let mut s = session();
        s.type_char('x');
        s.submit().expect("submitted");
        s.paste_text("one\ntwo\nthree\n");
        assert!(s.queue(), "the prompt was not queued");

        let entry = s.history.entries().last().expect("an entry");
        assert_eq!(entry.prompt, "one\ntwo\nthree\n");
    }

    /// A prompt coming back for editing comes back as it was typed. Returned as its words, the
    /// stack trace somebody folded away fills the box they are about to edit.
    #[test]
    fn a_stopped_turn_puts_a_folded_paste_back_behind_its_marker() {
        let mut s = session();
        for c in "what is ".chars() {
            s.type_char(c);
        }
        s.paste_text("one\ntwo\nthree\n");
        let prompt = s.submit().expect("submitted");

        s.restore(prompt);
        assert_eq!(s.input, "what is [Pasted text #1 +3 lines]");
    }

    /// Deleting the marker is the only way a user has to take a paste back, so it has to be the
    /// whole of the way: the words must not follow a marker no longer in the line.
    #[test]
    fn deleting_the_marker_takes_the_paste_back() {
        let mut s = session();
        s.paste_text("one\ntwo\nthree\n");
        for _ in 0.."[Pasted text #1 +3 lines]".len() {
            s.backspace();
        }
        for c in "never mind".chars() {
            s.type_char(c);
        }

        let prompt = s.submit().expect("submitted");
        assert_eq!(prompt, "never mind");
    }

    /// A command runs exactly as it is written, so a paste into shell mode is never folded: a
    /// line that is not what the user is looking at is the one thing that mode may never have.
    #[test]
    fn a_paste_into_a_command_line_is_never_folded() {
        let mut s = session();
        s.type_char('!');
        s.paste_text("one\ntwo\nthree\n");

        assert!(s.shell, "the paste left shell mode");
        assert_eq!(s.input, "one\ntwo\nthree\n");
    }

    /// One counter for everything a line can carry, so no two markers in front of a user can be
    /// numbered the same and a number always means one thing.
    #[test]
    fn a_paste_and_a_picture_never_share_a_number() {
        let mut s = session();
        s.attach(picture(b"pixels"));
        s.paste_text("one\ntwo\nthree\n");

        assert_eq!(s.input, "[Image #1][Pasted text #2 +3 lines]");
    }

    /// A picture is the session's own and nothing durable stands for it, so the prompt is
    /// remembered without the marker. Kept, it comes back in a later session naming a screenshot
    /// nobody can produce, and `[Image #1]` reaches the planner standing for nothing.
    #[test]
    fn a_recalled_prompt_does_not_name_a_picture_that_went_with_the_line() {
        let mut s = session();
        for c in "what is wrong here ".chars() {
            s.type_char(c);
        }
        s.attach(picture(b"pixels"));
        assert_eq!(s.input, "what is wrong here [Image #1]");
        s.submit().expect("submitted");

        let entry = s.history.entries().last().expect("an entry");
        assert_eq!(entry.prompt, "what is wrong here");
    }

    /// The picture still goes with the turn that named it. What is remembered is a question about
    /// the next session, and must not change what this one sends.
    #[test]
    fn settling_a_marker_for_the_history_does_not_take_the_picture_off_the_turn() {
        let mut s = session();
        s.attach(picture(b"pixels"));
        let prompt = s.submit().expect("submitted");

        assert_eq!(prompt, "[Image #1]");
        assert_eq!(s.sent_pasted().len(), 1);
    }

    /// A prompt recalled out of the history comes back as the words themselves, which is what
    /// makes it recallable at all: the marker it was typed behind stands for text that only the
    /// session holding it could put back.
    #[test]
    fn a_recalled_prompt_carries_the_words_that_were_pasted_into_it() {
        let mut s = session();
        s.paste_text("one\ntwo\nthree\n");
        s.submit().expect("submitted");
        s.complete("three lines", Vec::new(), 0);

        s.recall_older();
        assert_eq!(s.input, "one\ntwo\nthree\n");
        assert_eq!(s.submit().expect("submitted"), "one\ntwo\nthree");
    }

    /// A line with no marker in it is nobody's paste, and putting one back must not rewrite words
    /// that were typed.
    #[test]
    fn a_line_that_names_no_paste_is_sent_as_it_was_typed() {
        let s = session();
        assert_eq!(
            s.unfolded("[Pasted text #1 +3 lines]"),
            "[Pasted text #1 +3 lines]"
        );
    }

    /// A paste lands where typing does, so half a typed line plus a paste is one prompt.
    #[test]
    fn a_paste_joins_what_was_already_typed() {
        let mut s = session();
        s.type_char('>');
        s.paste(" pasted");
        assert_eq!(s.input, "> pasted");
    }

    /// A paste mid-turn is kept for the same reason typing is: it is the user's own words, and
    /// the only thing that must wait is sending them.
    #[test]
    fn a_paste_during_a_turn_is_kept() {
        let mut s = session();
        s.type_char('x');
        s.submit();
        assert_eq!(s.status, Status::Working);

        s.paste("more");
        assert_eq!(s.input, "more", "a paste was dropped mid-turn");
        assert!(s.submit().is_none(), "a second turn was allowed to start");
    }

    /// The editor was opened on the line, so what comes back is that line after thinking about
    /// it. Appending would give the user their own prompt twice.
    #[test]
    fn a_line_from_the_editor_replaces_what_was_typed() {
        let mut s = session();
        s.paste("half a thought");
        s.take_edited("a whole one, at last");
        assert_eq!(s.input, "a whole one, at last");
    }

    /// A recalled prompt taken through an editor is the working line now, exactly as it would be
    /// after a keystroke. Left browsing, the next Up would step away from the edit.
    /// The search shows when a prompt was sent and can narrow to the workspace it was sent from,
    /// so both have to be recorded as it is sent: neither can be worked out afterwards.
    #[test]
    fn a_sent_prompt_records_when_and_where_it_was_sent() {
        let mut session = Session::new("none").in_workspace("/work/here");
        for c in "why is this slow?".chars() {
            session.type_char(c);
        }
        session.submit().expect("submitted");

        let entry = session.history.entries().last().expect("an entry");
        assert_eq!(entry.prompt, "why is this slow?");
        assert_eq!(entry.project.as_deref(), Some("/work/here"));
        assert!(entry.at.is_some(), "no time was recorded");
    }

    #[test]
    fn editing_a_recalled_prompt_stops_browsing_history() {
        let mut s = session();
        s.history.push("an older prompt".to_string(), None);
        s.recall_older();
        assert!(s.history.is_browsing());

        s.take_edited("an older prompt, revised");
        assert!(!s.history.is_browsing(), "still browsing after an edit");
        assert_eq!(s.input, "an older prompt, revised");
    }

    /// A gauge reading zero before anything has been sent would be a claim about a context
    /// nobody has counted, in a session that has not started.
    #[test]
    fn nothing_is_said_about_the_context_until_a_request_has_been_measured() {
        assert_eq!(Session::new("none").fullness(), None);
    }

    #[test]
    fn how_full_the_context_is_comes_back_as_a_percentage() {
        let mut s = Session::new("none");
        s.measured(25_000, 100_000, false);
        assert_eq!(s.fullness(), Some(25));
    }

    /// The budget is a guess at a window nobody reports, so a request larger than it is a session
    /// about to be compacted rather than a context a hundred and forty per cent full.
    #[test]
    fn a_request_past_the_budget_reads_as_full_rather_than_more_than_full() {
        let mut s = Session::new("none");
        s.measured(140_000, 100_000, false);
        assert_eq!(s.fullness(), Some(100));
    }

    /// After a compaction nothing has been counted for the shortened conversation, and the old
    /// figure describes an exchange that is no longer being sent. Better to say nothing until the
    /// next turn counts it than to show a percentage that is no longer about anything.
    #[test]
    fn a_context_measured_at_nothing_is_a_context_nobody_has_measured() {
        let mut s = Session::new("none");
        s.measured(0, 100_000, false);
        assert_eq!(s.fullness(), None);
    }

    #[test]
    fn a_compacted_session_reports_compacted_occupancy() {
        let mut s = Session::new("none");
        s.compacted();
        assert_eq!(s.occupancy(), Occupancy::Compacted);
        assert_eq!(s.fullness(), None);
    }

    /// Picking a model the listing does not describe leaves the budget where it was and stops it
    /// being a window anybody reported, so the figure has to start saying it is approximate even
    /// though the arithmetic behind it did not move.
    #[test]
    fn a_budget_that_did_not_move_can_still_stop_being_one_anybody_advertised() {
        let mut s = Session::new("none");
        s.measured(20_000, 100_000, false);

        s.update_budget(100_000, true);
        assert_eq!(
            s.occupancy(),
            Occupancy::Measured {
                used: 20_000,
                budget: 100_000,
                guessed: true,
            }
        );
    }

    #[test]
    fn updating_budget_retains_token_count_with_new_capacity() {
        let mut s = Session::new("none");
        s.measured(20_000, 24_000, true);
        assert_eq!(s.fullness(), Some(83));

        s.update_budget(100_000, false);
        assert_eq!(
            s.occupancy(),
            Occupancy::Measured {
                used: 20_000,
                budget: 100_000,
                guessed: false,
            }
        );
        assert_eq!(s.fullness(), Some(20));
    }

    /// Before the first turn there is no turn to report, and a line claiming one would be about
    /// nothing.
    #[test]
    fn a_session_that_has_not_run_a_turn_reports_none_finished() {
        assert_eq!(Session::new("none").finished, None);
    }

    /// The case this exists for: a turn whose reply asked for no tool is over, and the spinner
    /// going out was the only thing that used to say so.
    #[test]
    fn a_completed_turn_is_reported_with_what_it_cost() {
        let mut s = Session::new("none");
        s.set_input("do the thing".to_string());
        s.submit();
        s.complete("now let me look at the dispatch code", Vec::new(), 4_200);

        let finished = s.finished.expect("a finished turn");
        assert_eq!(finished.turn, 1);
        assert_eq!(finished.tokens, 4_200);
        assert!(!finished.failed);
    }

    /// A tick beside a turn that did not finish would be the wrong thing to say about it.
    #[test]
    fn a_failed_turn_is_reported_as_failed() {
        let mut s = Session::new("none");
        s.set_input("do the thing".to_string());
        s.submit();
        s.fail("the model could not be reached");

        let finished = s.finished.expect("a finished turn");
        assert!(finished.failed);
    }

    /// A line reporting a finished turn while the next one runs is a line about the wrong turn.
    #[test]
    fn starting_another_turn_forgets_the_last_one() {
        let mut s = Session::new("none");
        s.set_input("do the thing".to_string());
        s.submit();
        s.complete("done", Vec::new(), 100);
        assert!(s.finished.is_some());

        s.set_input("do another thing".to_string());
        s.submit();
        assert_eq!(s.finished, None, "the last turn's figures outlived it");
    }

    /// A new session has not run the old one's turns.
    #[test]
    fn clearing_a_session_forgets_the_turn_that_finished() {
        let mut s = Session::new("none");
        s.set_input("do the thing".to_string());
        s.submit();
        s.complete("done", Vec::new(), 100);
        s.clear();
        assert_eq!(s.finished, None);
    }

    /// A new session's context is empty, so the gauge from the old one would be describing a
    /// conversation that no longer exists.
    #[test]
    fn clearing_a_session_forgets_how_full_the_old_one_was() {
        let mut s = Session::new("none");
        s.measured(90_000, 100_000, false);
        s.clear();
        assert_eq!(s.occupancy(), Occupancy::Unmeasured);
        assert_eq!(s.fullness(), None);
    }

    /// Compacting is not a turn: it adds nothing to the transcript, and a turn count that moved
    /// for it would make the next turn look like the one after two.
    #[test]
    fn an_aside_works_without_becoming_a_turn() {
        let mut s = Session::new("none");
        s.begin_aside();
        assert_eq!(s.status, Status::Working);
        s.end_aside(400);

        assert_eq!(s.status, Status::Idle);
        assert_eq!(s.turns, 0);
        assert_eq!(s.tokens, 400);
        assert!(s.transcript.is_empty());
    }

    #[test]
    fn submitting_returns_the_prompt_and_records_it() {
        let mut s = session();
        for c in "explain this".chars() {
            s.type_char(c);
        }
        assert_eq!(s.submit().as_deref(), Some("explain this"));
        assert!(s.input.is_empty());
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript[0].speaker, Speaker::User);
        assert_eq!(s.status, Status::Working);
    }

    /// A loop starts by doing the thing, not by waiting to. Somebody who has just asked for
    /// something every five minutes wants to see it happen once before they decide it is right.
    #[test]
    fn the_first_tick_of_a_loop_goes_immediately() {
        let mut s = session();
        let request = crate::loops::parse("5m check the deploy").expect("a request");

        assert_eq!(s.start_loop(request).as_deref(), Some("check the deploy"));
        assert_eq!(s.status, Status::Working);
        assert_eq!(
            s.transcript
                .iter()
                .filter(|entry| entry.speaker == Speaker::User)
                .count(),
            1
        );
    }

    /// A tick comes from a timer, and a command comes from a key press. So a loop over a line
    /// that reads like a command sends the characters, and this program acts on none of them.
    #[test]
    fn a_loop_whose_prompt_looks_like_a_command_still_sends_it_as_a_prompt() {
        let mut s = session();
        let request = crate::loops::parse("5m /status").expect("a request");

        assert_eq!(s.start_loop(request).as_deref(), Some("/status"));
        assert_eq!(s.looping().expect("a loop").prompt(), "/status");
    }

    /// A schedule is a request to be asked again, not a licence to interrupt. A tick that fired
    /// into a running turn would share its state with a turn nobody had finished reading.
    #[test]
    fn a_tick_waits_for_the_turn_in_flight_and_for_what_is_queued() {
        let mut s = session();
        s.start_loop(crate::loops::parse("5m watch").expect("a request"));
        s.complete("done", Vec::new(), 0);

        // Due, but the person has started something of their own.
        for c in "their own question".chars() {
            s.type_char(c);
        }
        s.submit();
        assert!(s.loop_tick().is_none(), "a tick interrupted a running turn");

        for c in "and another".chars() {
            s.type_char(c);
        }
        s.queue();
        s.complete("done", Vec::new(), 0);
        assert!(s.loop_tick().is_none(), "a tick jumped the queue");
    }

    /// The person's own prompt is not a tick of the loop, so finishing it must not re-arm one.
    /// Otherwise a loop would keep time from whatever its user happened to be doing.
    #[test]
    fn a_prompt_typed_during_a_loop_is_not_a_tick_of_it() {
        let mut s = session();
        s.start_loop(crate::loops::parse("watch").expect("a request"));
        assert!(s.looping().expect("a loop").ticking());
        s.complete("done", Vec::new(), 0);
        s.loop_turn_ended(Some(crate::loops::Wakeup::asked(120, false)));

        for c in "their own question".chars() {
            s.type_char(c);
        }
        s.submit();
        assert!(!s.looping().expect("a loop").ticking());
        s.complete("done", Vec::new(), 0);
        s.loop_turn_ended(None);

        assert!(
            s.looping().is_some(),
            "a prompt of their own ended the loop"
        );
    }

    /// A tick that says when to wake arms the next one, and the wait it named is the wait.
    #[test]
    fn a_tick_that_says_when_to_wake_arms_the_next_one() {
        let mut s = session();
        s.start_loop(crate::loops::parse("watch the build").expect("a request"));
        s.complete("done", Vec::new(), 0);
        s.loop_turn_ended(Some(crate::loops::Wakeup::asked(900, false)));

        let running = s.looping().expect("a loop");
        assert!(!running.ticking());
        assert!(
            running
                .until(std::time::Instant::now())
                .is_some_and(|until| until > std::time::Duration::from_secs(880)),
            "the next tick was not armed for the wait the turn named"
        );
    }

    /// The loop belongs to the conversation it was started in. Surviving into a cleared session
    /// it would send a prompt whose context has been thrown away.
    #[test]
    fn clearing_the_session_ends_the_loop() {
        let mut s = session();
        s.start_loop(crate::loops::parse("5m watch").expect("a request"));
        s.clear();
        assert!(s.looping().is_none());
    }

    #[test]
    fn stopping_a_loop_says_so_and_says_nothing_when_there_was_none() {
        let mut s = session();
        assert!(!s.stop_loop());
        assert!(s.transcript.is_empty());

        s.start_loop(crate::loops::parse("5m watch").expect("a request"));
        assert!(s.stop_loop());
        assert!(s.looping().is_none());
    }

    #[test]
    fn empty_input_does_not_submit() {
        let mut s = session();
        assert!(s.submit().is_none());
        s.type_char(' ');
        assert!(s.submit().is_none());
        assert_eq!(s.status, Status::Idle);
    }

    /// What a user types during a turn is kept, and still cannot start a second one.
    ///
    /// The typing used to be dropped, so a user writing during a slow turn watched their words
    /// go nowhere and had nothing on the screen to tell them why. Refusing to *send* is what
    /// keeps two turns from ever being in flight; refusing to accept the letters bought nothing.
    #[test]
    fn typing_during_a_turn_is_kept_but_cannot_send() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.submit();
        assert_eq!(s.status, Status::Working);

        for c in "second".chars() {
            s.type_char(c);
        }
        assert_eq!(s.input, "second", "typing was dropped mid-turn");
        assert!(s.submit().is_none(), "a second turn was allowed to start");
        assert_eq!(s.input, "second", "a refused send took the line with it");
    }

    /// A cancelled turn puts its prompt back, but not over the top of something the user typed
    /// while it ran. The newer of the two is the one they meant.
    #[test]
    fn a_cancelled_turn_does_not_overwrite_what_was_typed_meanwhile() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        let prompt = s.submit().expect("a prompt");

        for c in "wait".chars() {
            s.type_char(c);
        }
        s.restore(prompt);

        assert_eq!(s.input, "wait", "the restored prompt overwrote the typing");
        assert_eq!(s.status, Status::Idle);
    }

    #[test]
    fn completing_a_turn_returns_to_idle() {
        let mut s = session();
        s.type_char('a');
        s.submit();
        s.complete("the reply", Vec::new(), 0);

        assert_eq!(s.status, Status::Idle);
        assert_eq!(s.transcript.len(), 2);
        assert_eq!(s.transcript[1].speaker, Speaker::Assistant);
    }

    #[test]
    fn a_failure_also_returns_to_idle() {
        let mut s = session();
        s.type_char('a');
        s.submit();
        s.fail("something went wrong");

        assert_eq!(s.status, Status::Idle);
        assert_eq!(s.transcript[1].speaker, Speaker::System);
    }

    #[test]
    fn a_completed_turn_keeps_its_trail() {
        let mut s = session();
        s.type_char('a');
        s.submit();
        s.complete(
            "reply",
            vec![Event::Observed {
                capability: bravebot_core::capability::Capability::FileRead,
                label: Label::untrusted_private(),
            }],
            0,
        );
        assert_eq!(s.transcript[1].trail.len(), 1);
    }

    #[test]
    fn the_trail_can_be_toggled() {
        let mut s = session();
        assert!(!s.show_trail);
        s.toggle_trail();
        assert!(s.show_trail);
        s.toggle_trail();
        assert!(!s.show_trail);
    }

    #[test]
    fn scrolling_does_not_underflow() {
        let mut s = session();
        s.scroll_down(5);
        assert_eq!(s.scroll, 0);
        s.scroll_up(3);
        assert_eq!(s.scroll, 3);
        s.scroll_down(10);
        assert_eq!(s.scroll, 0);
    }

    #[test]
    fn submitting_resets_the_scroll_position() {
        let mut s = session();
        s.scroll_up(10);
        s.type_char('a');
        s.submit();
        assert_eq!(s.scroll, 0, "a new turn should return to the latest output");
    }

    #[test]
    fn quitting_is_observable() {
        let mut s = session();
        assert!(!s.is_quitting());
        s.quit();
        assert!(s.is_quitting());
    }

    /// No choice means the configured default applies, which is not the same as choosing a model
    /// named "": the turn has to be able to tell those apart.
    /// A snapshot describes the session as it stood before the last turn. Once something other
    /// than a turn has changed the session, rewinding to it would undo that change too, under a
    /// line saying one turn was rewound.
    #[test]
    fn closing_the_rewind_window_leaves_nothing_to_rewind_to() {
        let mut s = session();
        s.previous_turn = Some(TurnSnapshot {
            conversation: bravebot_agent::Conversation::new().snapshot(),
            turns: 1,
            tokens: 10,
            spend: std::collections::BTreeMap::new(),
            timing: std::collections::BTreeMap::new(),
            trust: bravebot_core::trust::TrustStore::new(),
            programs: bravebot_core::programs::TrustedPrograms::default(),
            transcript_len: 0,
            title: "a session".to_string(),
            was_wrote: true,
        });
        s.last_turn_backups.push(bravebot_agent::workspace::Backup {
            path: std::path::PathBuf::from("/tmp/whatever"),
            was: bravebot_agent::workspace::Before::Nothing,
        });

        s.close_rewind_window();

        assert!(s.previous_turn.is_none());
        assert!(s.last_turn_backups.is_empty());
    }

    /// Clearing drops the exchange, which is the whole point: a fresh context.
    #[test]
    fn clearing_drops_the_transcript_and_what_it_spent() {
        let mut s = session();
        s.type_char('a');
        s.submit();
        s.complete("an answer", Vec::new(), 500);
        assert!(!s.transcript.is_empty());

        s.clear();
        assert!(s.transcript.is_empty(), "the transcript survived");
        assert_eq!(s.turns, 0);
        assert_eq!(s.tokens, 0, "the spend survived");
        assert_eq!(s.status, Status::Idle);
        assert!(
            s.previous_turn.is_none(),
            "the previous turn survived clear"
        );
        assert!(s.last_turn_backups.is_empty(), "the backups survived clear");
    }

    /// What belongs to the user rather than to the session survives, since none of it is a
    /// permission over the workspace and re-asking would forget something told once. The trust map
    /// is the opposite case and is not kept, but it does not live here: the loop drops it and asks
    /// again.
    #[test]
    fn clearing_keeps_what_the_user_chose() {
        let mut s = session();
        s.choose_model("claude-3-sonnet");
        s.type_char('a');
        s.submit();
        s.complete("an answer", Vec::new(), 10);

        s.clear();
        assert_eq!(
            s.model(),
            Some("claude-3-sonnet"),
            "the model was forgotten"
        );
        assert_eq!(s.confinement, "kernel-enforced");
        assert_eq!(s.history.len(), 1, "the prompt history was dropped");
    }

    /// A prompt half-typed when the user cleared is still a prompt they meant to send.
    #[test]
    fn clearing_leaves_the_input_line_alone() {
        let mut s = session();
        for c in "half a thought".chars() {
            s.type_char(c);
        }
        s.clear();
        assert_eq!(s.input, "half a thought");
    }

    /// Nothing belonging to the turn just finished may appear beneath the next one's work.
    ///
    /// Asserted between turns rather than during one, because that is the only moment `/clear` can
    /// happen: a running turn does not accept Enter, so the line waits until it ends.
    #[test]
    fn clearing_forgets_the_previous_turn() {
        let mut s = session();
        s.type_char('a');
        s.submit();
        s.set_todos(bravebot_core::todo::rows(&bravebot_core::todo::List::new(
            vec![bravebot_core::todo::Item::new(
                "something",
                bravebot_core::todo::Status::Active,
            )],
        )));
        s.complete("an answer", Vec::new(), 10);
        s.scroll_up(5);

        s.clear();
        assert!(s.todos.is_empty(), "a task list outlived the turn");
        assert!(s.indicator().is_none(), "the indicator outlived the turn");
        assert_eq!(s.scroll, 0, "the scroll position outlived the transcript");
        assert!(
            s.todos_by_turn().is_empty(),
            "a finished turn's list would still be written to the new session"
        );
    }

    /// A report reads as a block, so both columns line up: an aside starting wherever its value
    /// happened to end is harder to read than no column at all.
    #[test]
    fn a_report_lines_its_columns_up() {
        let mut s = session();
        s.report(crate::status::Report {
            lines: vec![
                crate::status::Line {
                    label: "Model".to_string(),
                    value: "a-long-model-name".to_string(),
                    note: "chosen".to_string(),
                },
                crate::status::Line {
                    label: "Endpoint".to_string(),
                    value: "dev".to_string(),
                    note: "premium".to_string(),
                },
            ],
        });

        let notes: Vec<&str> = s.transcript.iter().map(|e| e.text.as_str()).collect();
        let column_of = |line: &str, word: &str| line.find(word).expect("the word is on the line");
        assert_eq!(
            column_of(notes[0], "a-long-model-name"),
            column_of(notes[1], "dev"),
            "the values did not line up: {notes:?}"
        );
        assert_eq!(
            column_of(notes[0], "chosen"),
            column_of(notes[1], "premium"),
            "the notes did not line up: {notes:?}"
        );
    }

    /// A row with no aside must not have its value padded, or one long value would push every note
    /// across the screen.
    #[test]
    fn a_row_with_no_note_is_not_padded() {
        let mut s = session();
        s.report(crate::status::Report {
            lines: vec![crate::status::Line {
                label: "Session".to_string(),
                value: "a name".to_string(),
                note: String::new(),
            }],
        });
        assert_eq!(s.transcript[0].text, "Session  a name");
    }

    #[test]
    fn a_session_starts_with_no_model_chosen() {
        assert_eq!(session().model(), None);
    }

    #[test]
    fn choosing_a_model_is_observable() {
        let mut s = session();
        s.choose_model("claude-3-sonnet");
        assert_eq!(s.model(), Some("claude-3-sonnet"));
    }

    /// Asking is what a session opens in, which is what every session did before a mode could be
    /// chosen: the mode nobody picked cannot be one that stops putting writes to a person.
    #[test]
    fn a_session_starts_by_asking_about_everything() {
        assert_eq!(
            session().permission_mode(),
            bravebot_agent::PermissionMode::Ask
        );
    }

    /// A mode belongs to the sitting somebody chose it in, so cycling one is not a change to the
    /// session's saved state. What a resume restores is the record, and the mode is not part of it.
    #[test]
    fn cycling_the_mode_changes_nothing_a_resume_would_read() {
        let mut session = session();
        session.cycle_permission_mode();
        assert_ne!(
            session.permission_mode(),
            bravebot_agent::PermissionMode::Ask,
            "the mode did not move, so this test proves nothing"
        );
        // Everything a session hands the record, unmoved by the press above.
        assert_eq!(session.turns, 0);
        assert_eq!(session.tokens, 0);
        assert!(session.todos.is_empty());
        assert_eq!(session.model(), None);
    }
    /// The indicator only exists while a turn is in flight.
    #[test]
    fn the_indicator_appears_only_while_working() {
        let mut s = session();
        assert!(s.indicator().is_none());
        s.type_char('a');
        s.submit();
        assert!(s.indicator().is_some());
        s.complete("reply", Vec::new(), 0);
        assert!(s.indicator().is_none());
    }

    /// Each turn advances the word, so a new turn is visibly a new turn.
    #[test]
    fn each_turn_gets_a_different_word() {
        let mut s = session();
        s.type_char('a');
        s.submit();
        let first = s.indicator().expect("working").verb;
        s.complete("r", Vec::new(), 0);

        s.type_char('b');
        s.submit();
        let second = s.indicator().expect("working").verb;
        assert_ne!(first, second);
    }

    /// The count is for the session, not the last turn: the question it answers is what the
    /// whole conversation has cost.
    #[test]
    fn tokens_accumulate_across_turns() {
        let mut s = session();
        s.type_char('a');
        s.submit();
        s.complete("r", Vec::new(), 1_000);
        assert_eq!(s.tokens, 1_000);

        s.type_char('b');
        s.submit();
        s.complete("r", Vec::new(), 500);
        assert_eq!(s.tokens, 1_500);
    }

    /// A failed turn must stop the clock, or an idle session would keep counting.
    #[test]
    fn a_failure_stops_the_clock() {
        let mut s = session();
        s.type_char('a');
        s.submit();
        s.fail("error");
        assert_eq!(s.elapsed(), Duration::ZERO);
        assert!(s.indicator().is_none());
    }

    #[test]
    fn an_idle_session_has_no_elapsed_time() {
        assert_eq!(session().elapsed(), Duration::ZERO);
    }
    #[test]
    fn clearing_discards_the_input() {
        let mut s = session();
        for c in "hello".chars() {
            s.type_char(c);
        }
        s.clear_input();
        assert!(s.input.is_empty());
    }

    /// Input belongs to the idle state. Every other editing method is guarded the same way, and
    /// an unguarded clear would let a stray key empty a field the user had not touched.
    #[test]
    fn clearing_is_refused_while_a_turn_is_running() {
        let mut s = session();
        for c in "kept".chars() {
            s.type_char(c);
        }
        s.submit();
        // `submit` takes the text, and typing during the turn is how a user puts more back.
        for c in "mid-turn".chars() {
            s.type_char(c);
        }

        s.clear_input();
        assert_eq!(s.input, "mid-turn", "the input was cleared mid-turn");
    }
    /// Cancelling returns the prompt for editing, which is the point of cancelling rather than
    /// waiting: the text is not lost.
    #[test]
    fn restoring_puts_the_prompt_back_and_returns_to_idle() {
        let mut s = session();
        for c in "half an idea".chars() {
            s.type_char(c);
        }
        let prompt = s.submit().expect("submitted");
        assert_eq!(s.status, Status::Working);

        s.restore(prompt);

        assert_eq!(s.input, "half an idea");
        assert_eq!(s.status, Status::Idle);
        assert!(s.indicator().is_none(), "the indicator kept running");
    }

    /// The cancelled prompt is removed from the transcript: it produced nothing, and leaving it
    /// would read as a question that went unanswered.
    #[test]
    fn restoring_removes_the_unanswered_prompt() {
        let mut s = session();
        for c in "question".chars() {
            s.type_char(c);
        }
        let prompt = s.submit().expect("submitted");
        assert_eq!(s.transcript.len(), 1);

        s.restore(prompt);
        assert!(
            s.transcript.is_empty(),
            "the prompt was left in the transcript"
        );
    }

    /// Earlier exchanges are untouched, so cancelling does not eat the conversation.
    #[test]
    fn restoring_keeps_earlier_exchanges() {
        let mut s = session();
        s.type_char('a');
        let first = s.submit().expect("submitted");
        s.complete("an answer", Vec::new(), 0);
        assert_eq!(s.transcript.len(), 2);
        let _ = first;

        s.type_char('b');
        let second = s.submit().expect("submitted");
        s.restore(second);

        assert_eq!(s.transcript.len(), 2, "an earlier exchange was removed");
    }
    /// A submitted prompt is recallable afterwards.
    #[test]
    fn submitting_records_the_prompt_in_history() {
        let mut s = session();
        for c in "a question".chars() {
            s.type_char(c);
        }
        s.submit().expect("submitted");
        s.complete("answer", Vec::new(), 0);

        assert_eq!(s.history.len(), 1);
        s.recall_older();
        assert_eq!(s.input, "a question");
    }

    /// A cancelled prompt goes back into the box, so it must leave history rather than being
    /// offered from two places at once.
    #[test]
    fn cancelling_pops_the_prompt_from_history() {
        let mut s = session();
        for c in "abandoned".chars() {
            s.type_char(c);
        }
        let prompt = s.submit().expect("submitted");
        assert_eq!(s.history.len(), 1);

        s.restore(prompt);
        assert_eq!(s.input, "abandoned");
        assert!(
            s.history.is_empty(),
            "the cancelled prompt stayed in history"
        );
    }

    /// Enter mid-turn used to do nothing at all: the line sat in the box until the person
    /// noticed the turn had ended and pressed it again. It goes now, and waits its turn.
    #[test]
    fn a_prompt_sent_while_a_turn_runs_waits_for_it() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.submit().expect("submitted");

        for c in "second".chars() {
            s.type_char(c);
        }
        assert!(s.queue(), "the line was not taken");
        assert!(s.input.is_empty(), "the line stayed in the box");
        assert_eq!(s.queued.len(), 1);

        // Still one turn: queueing is not starting.
        assert_eq!(s.turns, 1);
        assert_eq!(
            s.transcript.iter().filter(|e| e.text == "second").count(),
            0,
            "a prompt that has not been sent was written into the transcript"
        );
    }

    /// It goes when the turn it waited for is over, and not before. Sending it while the first
    /// was still running is the thing a running turn refuses.
    #[test]
    fn a_waiting_prompt_goes_when_the_turn_ends() {
        let mut s = session();
        s.type_char('a');
        s.submit().expect("submitted");
        for c in "second".chars() {
            s.type_char(c);
        }
        s.queue();

        assert!(
            s.send_queued().is_none(),
            "it went while a turn was running"
        );

        s.complete("an answer", Vec::new(), 0);
        assert_eq!(s.send_queued().as_deref(), Some("second"));
        assert_eq!(s.status, Status::Working);
        assert_eq!(s.turns, 2);
        assert!(s.queued.is_empty());
        assert!(s.send_queued().is_none(), "it went twice");
    }

    /// Typed in one order, sent in that order. A queue that reordered what somebody said would
    /// be worse than one that dropped it.
    #[test]
    fn waiting_prompts_go_in_the_order_they_were_typed() {
        let mut s = session();
        s.type_char('a');
        s.submit().expect("submitted");
        for line in ["second", "third"] {
            for c in line.chars() {
                s.type_char(c);
            }
            s.queue();
        }
        assert_eq!(s.queued.len(), 2);

        s.complete("an answer", Vec::new(), 0);
        assert_eq!(s.send_queued().as_deref(), Some("second"));
        s.complete("another", Vec::new(), 0);
        assert_eq!(s.send_queued().as_deref(), Some("third"));
    }

    /// A prompt with others waiting behind it stays sent. The box is about to be needed for the
    /// next of them, and un-sending this one would put a line back there while the turns it was
    /// sent before go on running, out of the order the person typed them in.
    #[test]
    fn a_stopped_prompt_stays_sent_where_others_are_waiting() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.submit().expect("submitted");
        for c in "second".chars() {
            s.type_char(c);
        }
        s.queue();

        s.restore("first");

        assert_eq!(s.input(), "", "the stopped prompt went back into the box");
        assert!(
            s.transcript
                .iter()
                .any(|entry| entry.speaker == Speaker::User && entry.text == "first"),
            "the stopped prompt was un-sent"
        );
        assert!(
            s.transcript
                .last()
                .is_some_and(|entry| entry.text == "stopped"),
            "nothing recorded that it stopped"
        );
    }

    /// And with nothing behind it there is nothing to keep the order of, so it is un-sent whole
    /// and comes back for editing, which is the point of stopping rather than waiting.
    #[test]
    fn a_stopped_prompt_comes_back_where_nothing_is_waiting() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.submit().expect("submitted");

        s.restore("first");

        assert_eq!(s.input(), "first");
    }

    /// A stop is aimed at the turn in flight. The prompts behind it are ones the person typed and
    /// has not taken back, and throwing them away made stopping a turn that had gone wrong cost
    /// every prompt they had queued while it did.
    #[test]
    fn stopping_a_turn_keeps_what_was_waiting_behind_it() {
        let mut s = session();
        s.type_char('a');
        s.submit().expect("submitted");
        for c in "second".chars() {
            s.type_char(c);
        }
        s.queue();
        for c in "third".chars() {
            s.type_char(c);
        }
        s.queue();

        s.restore("a");

        assert_eq!(s.queued.len(), 2, "the stop took the queue with it");
        assert_eq!(
            s.send_queued().as_deref(),
            Some("second"),
            "it did not go on"
        );
    }

    /// A line waiting to go is a line that was sent, so it is in the history like any other.
    #[test]
    fn a_waiting_prompt_is_in_the_history_already() {
        let mut s = session();
        s.type_char('a');
        s.submit().expect("submitted");
        for c in "second".chars() {
            s.type_char(c);
        }
        s.queue();

        assert_eq!(s.history.len(), 2, "the queued line was not remembered");
        s.recall_older();
        assert_eq!(s.input, "second");
    }

    /// Nothing to queue is not a queue of nothing, and with no turn running Enter sends rather
    /// than waits.
    #[test]
    fn there_is_nothing_to_queue_when_the_line_is_blank_or_nothing_is_running() {
        let mut s = session();
        s.type_char('a');
        s.submit().expect("submitted");
        assert!(!s.queue(), "a blank line was queued");

        s.complete("an answer", Vec::new(), 0);
        for c in "next".chars() {
            s.type_char(c);
        }
        assert!(!s.queue(), "queued with no turn to wait for");
        assert_eq!(s.input, "next", "the line was taken anyway");
    }

    /// Up is how a person reaches back for what they said last, and while something is waiting the
    /// last thing they said is in the queue rather than behind them. Recalling it instead handed
    /// back a copy: the copy was edited, and the original went as it was.
    #[test]
    fn taking_the_queue_back_puts_every_waiting_prompt_in_the_box() {
        let mut s = session();
        s.type_char('a');
        s.submit().expect("submitted");
        for line in ["second", "third"] {
            for c in line.chars() {
                s.type_char(c);
            }
            s.queue();
        }

        assert!(s.unqueue(), "nothing came back");
        assert_eq!(s.input, "second\nthird");
        assert_eq!(s.caret, s.input.len(), "the caret is not where typing goes");
        assert!(s.queued.is_empty(), "a prompt was left waiting");

        s.complete("an answer", Vec::new(), 0);
        assert!(
            s.send_queued().is_none(),
            "a prompt taken back was sent anyway"
        );
    }

    /// The line in the box was typed after the prompts that are waiting, so it stays after them,
    /// and it is where the caret was going to be. Dropped instead, taking the queue back would
    /// cost the person the sentence they were in the middle of.
    #[test]
    fn a_half_typed_line_stays_below_what_comes_back() {
        let mut s = session();
        s.type_char('a');
        s.submit().expect("submitted");
        for c in "waiting".chars() {
            s.type_char(c);
        }
        s.queue();
        for c in "half".chars() {
            s.type_char(c);
        }

        s.unqueue();
        assert_eq!(s.input, "waiting\nhalf");
    }

    /// A marker is text in the prompt, and what it stands for was taken off the staging list when
    /// the prompt was queued. Coming back without it, the line would name a picture that is no
    /// longer there and send a marker standing over nothing.
    #[test]
    fn what_a_waiting_prompt_named_is_named_again_when_it_comes_back() {
        let mut s = session();
        s.type_char('a');
        s.submit().expect("submitted");
        for c in "look at ".chars() {
            s.type_char(c);
        }
        s.attach(picture(b"pixels"));
        s.queue();
        assert!(
            s.pasted_named(&s.input).is_empty(),
            "the box still named it"
        );

        s.unqueue();
        assert_eq!(
            s.pasted_named(&s.input).len(),
            1,
            "the picture did not come back with the words"
        );
    }

    /// Nothing waiting is not a queue of nothing. With none the key means what it has always
    /// meant, and walks the history.
    #[test]
    fn there_is_nothing_to_take_back_when_nothing_is_waiting() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.submit().expect("submitted");

        assert!(!s.unqueue(), "something came back out of an empty queue");
        assert_eq!(s.input, "", "the box was rewritten anyway");
    }

    /// The box takes words while a turn runs, so it takes recalled ones too. Refusing here was
    /// left over from when it took nothing at all: a person could type their next prompt during a
    /// turn but not reach the one they had just sent, which is the one they most often want when
    /// a turn is going wrong in front of them.
    #[test]
    fn recall_works_while_a_turn_is_running() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.submit().expect("submitted");
        assert_eq!(s.status, Status::Working);

        s.recall_older();
        assert_eq!(s.input, "first", "history could not be reached mid-turn");

        s.recall_newer();
        assert!(s.input.is_empty(), "stepping forward did not come back");
    }

    /// Reaching a prompt is not sending one. Whatever is in the box, a second turn must not begin
    /// while the first is in flight.
    #[test]
    fn a_recalled_prompt_still_cannot_be_sent_while_a_turn_is_running() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.submit().expect("submitted");
        s.recall_older();

        assert!(s.submit().is_none(), "a second turn started mid-flight");
    }

    /// A list left up over a line that arrived under it belongs to a press two prompts ago. Every
    /// path that writes a whole line takes it down, and these two are the ones a person reaches
    /// while a turn is in flight: asking for the keys, then recalling a prompt or stopping the turn.
    #[test]
    fn a_line_that_arrives_under_the_list_takes_the_list_down() {
        let mut recalled = session();
        for c in "first".chars() {
            recalled.type_char(c);
        }
        recalled.submit().expect("submitted");
        recalled.type_char('?');
        assert!(recalled.shortcuts, "the list did not come up");

        recalled.recall_older();
        assert_eq!(recalled.input, "first");
        assert!(!recalled.shortcuts, "the list stands over a recalled line");

        let mut stopped = session();
        for c in "first".chars() {
            stopped.type_char(c);
        }
        stopped.submit().expect("submitted");
        stopped.type_char('?');
        assert!(stopped.shortcuts, "the list did not come up");

        stopped.restore("first".to_string());
        assert_eq!(stopped.input, "first");
        assert!(!stopped.shortcuts, "the list stands over a stopped prompt");
    }

    mod todos {
        use super::*;
        use bravebot_core::todo::{Item, List, Status, rows};

        fn list(entries: &[(&str, Status)]) -> Vec<bravebot_core::todo::Row> {
            rows(&List::new(
                entries
                    .iter()
                    .map(|(content, status)| Item::new(*content, *status))
                    .collect(),
            ))
        }

        fn working() -> Session {
            let mut s = session();
            s.type_char('a');
            s.submit();
            s
        }

        #[test]
        fn a_reported_list_is_kept_for_the_display() {
            let mut s = working();
            s.set_todos(list(&[("first", Status::Active)]));
            assert_eq!(s.todos.len(), 1);
        }

        /// An update replaces the previous list rather than adding to it, matching the tool: the
        /// model sends the whole list every time.
        #[test]
        fn a_later_report_replaces_the_earlier_one() {
            let mut s = working();
            s.set_todos(list(&[
                ("first", Status::Active),
                ("second", Status::Pending),
            ]));
            s.set_todos(list(&[("first", Status::Done), ("second", Status::Active)]));

            assert_eq!(s.todos.len(), 2);
            assert!(
                s.todos[0].struck(),
                "the first task did not get crossed off"
            );
        }

        /// An empty list must clear the display. Keeping the previous one would leave finished
        /// work on screen that the model has said is no longer its plan.
        #[test]
        fn an_empty_report_clears_the_display() {
            let mut s = working();
            s.set_todos(list(&[("something", Status::Active)]));
            s.set_todos(Vec::new());
            assert!(s.todos.is_empty());
        }

        /// The list belongs to the turn that reported it. A new turn starting with the previous
        /// turn's plan would show finished work as outstanding again.
        #[test]
        fn a_new_turn_starts_with_no_list() {
            let mut s = working();
            s.set_todos(list(&[("from the first turn", Status::Done)]));
            s.complete("done", Vec::new(), 0);

            s.type_char('b');
            s.submit();
            assert!(s.todos.is_empty(), "the previous turn's list carried over");
        }

        /// It moves onto the entry instead of being dropped, so the scrollback shows what each
        /// turn set out to do next to the answer it gave.
        #[test]
        fn a_finished_turn_keeps_its_list_in_the_transcript() {
            let mut s = working();
            s.set_todos(list(&[("a task", Status::Done)]));
            s.complete("the answer", Vec::new(), 0);

            let entry = s.transcript.last().expect("an entry");
            assert_eq!(entry.speaker, Speaker::Assistant);
            assert_eq!(entry.todos.len(), 1);
            assert!(s.todos.is_empty(), "the live list was not handed over");
        }

        /// A failed turn keeps its list too, unfinished. Three of five done is more useful shown
        /// than blank.
        #[test]
        fn a_failed_turn_keeps_its_unfinished_list() {
            let mut s = working();
            s.set_todos(list(&[
                ("done", Status::Done),
                ("not done", Status::Active),
            ]));
            s.fail("the model call failed");

            let entry = s.transcript.last().expect("an entry");
            assert_eq!(entry.todos.len(), 2);
            assert!(!entry.todos[1].struck());
        }

        /// A cancelled turn is being un-sent, so its plan goes with the prompt rather than
        /// staying on screen describing work nobody asked for.
        #[test]
        fn a_cancelled_turn_discards_its_list() {
            let mut s = session();
            for c in "a question".chars() {
                s.type_char(c);
            }
            let prompt = s.submit().expect("submitted");
            s.set_todos(list(&[("started this", Status::Active)]));

            s.restore(prompt);
            assert!(s.todos.is_empty(), "the cancelled turn's list stayed");
            assert!(s.transcript.is_empty());
        }

        /// The active task is drawn in the list, under the turn it belongs to, so it does not
        /// also take the word beside the spinner. That word is there to show the session is
        /// alive, and a list already says what the work is.
        #[test]
        fn the_active_task_is_shown_in_the_list_and_not_on_the_spinner() {
            let mut s = working();
            let word = s.indicator().expect("working").verb.to_string();
            s.set_todos(list(&[
                ("Escape cancels a turn", Status::Done),
                ("Add prompt history", Status::Active),
            ]));

            assert_eq!(s.indicator().expect("working").verb, word);
            assert!(
                s.todos
                    .iter()
                    .any(|row| row.content == "Add prompt history"),
                "the task went nowhere at all"
            );
        }

        /// With no list, or nothing active in it, the turn's own word is used: a session that
        /// never calls the tool must look exactly as it did before.
        #[test]
        fn without_an_active_task_the_indicator_keeps_its_own_word() {
            let mut s = working();
            let generic = s.indicator().expect("working").verb.to_string();

            s.set_todos(list(&[("all finished", Status::Done)]));
            assert_eq!(s.indicator().expect("working").verb, generic);
        }

        /// The written count reaches the indicator, which is the whole point of streaming it.
        #[test]
        fn the_written_count_reaches_the_indicator() {
            let mut s = working();
            assert!(s.indicator().expect("working").written.is_none());

            s.set_written(512);
            assert_eq!(
                s.indicator().expect("working").written.as_deref(),
                Some("512")
            );
        }

        /// It measures the reply being written now, so a new turn starts from nothing rather than
        /// continuing the previous turn's figure.
        #[test]
        fn a_new_turn_resets_the_written_count() {
            let mut s = working();
            s.set_written(900);
            s.complete("done", Vec::new(), 1_000);

            s.type_char('b');
            s.submit();
            assert_eq!(s.written, 0, "the previous turn's count carried over");
            assert!(s.indicator().expect("working").written.is_none());
        }

        /// The session total still accumulates, since it answers a different question: what the
        /// whole conversation has cost.
        #[test]
        fn the_session_total_still_accumulates_across_turns() {
            let mut s = working();
            s.set_written(100);
            s.complete("first", Vec::new(), 1_000);

            s.type_char('b');
            s.submit();
            s.set_written(50);
            s.complete("second", Vec::new(), 500);

            assert_eq!(s.tokens, 1_500);
        }
    }

    mod progress {
        use super::*;
        use bravebot_agent::report::{Activity, Phase};

        fn working() -> Session {
            let mut s = session();
            s.type_char('a');
            s.submit();
            s
        }

        /// The whole point: a call is on screen while it runs, not only once it is over.
        #[test]
        fn a_call_appears_before_it_finishes() {
            let mut s = working();
            s.start_activity(Activity::running("Read", "src/main.rs"));

            let entry = s.transcript.last().expect("an entry");
            assert_eq!(entry.speaker, Speaker::Tool);
            assert_eq!(entry.text, "Read(src/main.rs)");
            assert!(
                entry.activity.as_ref().expect("an activity").is_running(),
                "the call was recorded as already over"
            );
        }

        /// Finishing replaces the running line rather than adding a second one, or every call
        /// would appear twice.
        #[test]
        fn finishing_replaces_the_line_rather_than_adding_one() {
            let mut s = working();
            s.start_activity(Activity::running("Read", "src/main.rs"));
            s.finish_activity(Activity::running("Read", "src/main.rs").done("12 lines"));

            let tools: Vec<&Entry> = s
                .transcript
                .iter()
                .filter(|e| e.speaker == Speaker::Tool)
                .collect();
            assert_eq!(tools.len(), 1, "the call was recorded twice");
            assert_eq!(
                tools[0]
                    .activity
                    .as_ref()
                    .expect("an activity")
                    .note
                    .as_deref(),
                Some("12 lines")
            );
        }

        /// Several calls in a row each keep their own line.
        #[test]
        fn each_call_keeps_its_own_line() {
            let mut s = working();
            for path in ["a.rs", "b.rs", "c.rs"] {
                s.start_activity(Activity::running("Read", path));
                s.finish_activity(Activity::running("Read", path).done("1 line"));
            }
            assert_eq!(
                s.transcript
                    .iter()
                    .filter(|e| e.speaker == Speaker::Tool)
                    .count(),
                3
            );
        }

        /// A finish with nothing running is still recorded. Losing the record of a call that
        /// happened is worse than an unpaired line.
        #[test]
        fn a_finish_without_a_start_is_still_recorded() {
            let mut s = working();
            s.finish_activity(Activity::running("Write", "a.rs").done("3 lines"));
            assert_eq!(
                s.transcript.last().expect("an entry").speaker,
                Speaker::Tool
            );
        }

        /// The model's account of its own work is the best progress report there is.
        #[test]
        fn narration_lands_in_the_transcript_as_the_assistant() {
            let mut s = working();
            s.narrate("Let me look at the config first.");

            let entry = s.transcript.last().expect("an entry");
            assert_eq!(entry.speaker, Speaker::Assistant);
            assert_eq!(entry.text, "Let me look at the config first.");
        }

        /// The words arrive a fragment at a time and are one reply, so they accumulate rather
        /// than replace. Replacing left the screen showing whatever the last frame happened to
        /// carry, which for a long answer is the last three characters of it.
        #[test]
        fn a_streamed_reply_grows_rather_than_being_replaced() {
            let mut s = working();
            s.streaming("Let me look ");
            s.streaming("at the config ");
            s.streaming("first.");
            assert_eq!(s.streaming, "Let me look at the config first.");
        }

        /// The tail and the entry are the same words, so leaving the tail up would draw them
        /// twice: once as the reply arriving and once as the reply that arrived.
        #[test]
        fn the_finished_round_takes_over_from_the_reply_that_was_arriving() {
            let mut s = working();
            s.streaming("Let me look at the config first.");
            s.narrate("Let me look at the config first.");

            assert!(s.streaming.is_empty(), "the tail was drawn twice");
            assert_eq!(
                s.transcript.last().expect("an entry").text,
                "Let me look at the config first."
            );
        }

        /// A round that ends with nothing to say still has to take its tail down, and a round
        /// whose reply is the turn's answer does too. Left up, half a sentence sat under the
        /// finished answer for the rest of the session.
        #[test]
        fn a_reply_that_was_arriving_is_taken_down_however_the_round_ends() {
            let mut s = working();
            s.streaming("half a thought");
            s.narrate("");
            assert!(s.streaming.is_empty(), "a silent round left its tail up");

            s.streaming("half a thought");
            s.complete("the answer", Vec::new(), 0);
            assert!(s.streaming.is_empty(), "a finished turn left its tail up");

            s.streaming("half a thought");
            s.fail("error: something went wrong");
            assert!(s.streaming.is_empty(), "a failed turn left its tail up");
        }

        /// A model with nowhere else to put its working writes it into the reply and closes it
        /// before answering. The tail is the same words as the entry that replaces it, so what
        /// is held back from one is held back from the other.
        #[test]
        fn a_thought_arriving_is_not_drawn_at_the_tail() {
            let mut s = working();
            s.streaming("<think>they want the config path");
            assert_eq!(
                s.reply_so_far(),
                "",
                "the working was drawn as it was written"
            );

            s.streaming("</think>It is in ~/.bravebot.");
            assert_eq!(s.reply_so_far(), "It is in ~/.bravebot.");
        }

        /// The answer is what the turn produced and the block above it is the model talking to
        /// itself. Kept, it goes into the transcript, into the session record, and back onto the
        /// screen every time that session is resumed.
        #[test]
        fn a_finished_reply_keeps_the_answer_and_not_the_thought() {
            let mut s = working();
            s.complete(
                "<think>they want the config path</think>It is in ~/.bravebot.",
                Vec::new(),
                0,
            );
            assert_eq!(
                s.transcript.last().expect("an entry").text,
                "It is in ~/.bravebot."
            );
        }

        /// And the same for what a round says on its way to the next call.
        #[test]
        fn a_round_that_thought_before_speaking_records_only_what_it_said() {
            let mut s = working();
            s.narrate("<think>read the config first</think>Let me look at the config.");
            assert_eq!(
                s.transcript.last().expect("an entry").text,
                "Let me look at the config."
            );
        }

        /// A round whose whole narration was a thought said nothing, and a blank entry among the
        /// calls it made reads as a turn that lost its words.
        #[test]
        fn a_round_that_only_thought_leaves_no_entry() {
            let mut s = working();
            let before = s.transcript.len();
            s.narrate("<think>nothing worth saying yet</think>");
            assert_eq!(s.transcript.len(), before);
        }

        /// What a request that was thrown away had written is not part of the reply that
        /// replaces it, and a phase is announced at the top of every round and on every retry.
        #[test]
        fn a_round_starting_afresh_starts_from_an_empty_tail() {
            let mut s = working();
            s.streaming("this reply was abandoned");
            s.set_phase(Phase::Reconnecting);
            assert!(s.streaming.is_empty());
        }

        /// A round with no prose still reports, so the blank has to be dropped here: an empty
        /// entry would draw as a gap the user cannot account for.
        #[test]
        fn empty_narration_is_not_drawn() {
            let mut s = working();
            let before = s.transcript.len();
            s.narrate("");
            s.narrate("   \n  ");
            assert_eq!(s.transcript.len(), before);
        }

        /// The call in flight has a line of its own in the transcript, so putting it beside the
        /// spinner as well said it twice and made the word odd: "Isolated processor(index.html,
        /// server.py)…" is a strange thing to read there. The word's job is showing the session
        /// is alive while an answer takes its time.
        #[test]
        fn a_running_call_leaves_the_turn_its_own_word() {
            let mut s = working();
            s.set_phase(Phase::Thinking);
            let word = s.indicator().expect("working").verb.to_string();
            s.start_activity(Activity::running("Search", "MAX_STEPS"));
            assert_eq!(s.indicator().expect("working").verb, word);
        }

        /// The first wait is the long one and has no call to show for it, so the phase word is
        /// what stops it reading as a hang.
        #[test]
        fn the_phase_names_the_indicator_when_nothing_else_can() {
            let mut s = working();
            let generic = s.indicator().expect("working").verb.to_string();
            s.set_phase(Phase::Planning);
            assert_eq!(s.indicator().expect("working").verb, "Planning");
            assert_ne!(generic, "Planning");
        }

        /// The task in hand is drawn under the turn, in the list, so it does not take the word
        /// either.
        #[test]
        fn an_active_task_leaves_the_turn_its_own_word() {
            let mut s = working();
            s.set_phase(Phase::Thinking);
            let word = s.indicator().expect("working").verb.to_string();
            s.set_todos(bravebot_core::todo::rows(&bravebot_core::todo::List::new(
                vec![bravebot_core::todo::Item::new(
                    "Add prompt history",
                    bravebot_core::todo::Status::Active,
                )],
            )));
            assert_eq!(s.indicator().expect("working").verb, word);
        }

        /// Planning and reconnecting do take it. The first is the wait before anything at all
        /// has appeared, and the second is a pause that looks exactly like thinking and is not.
        #[test]
        fn the_phases_worth_naming_name_the_indicator() {
            let mut s = working();
            s.set_phase(Phase::Planning);
            assert_eq!(s.indicator().expect("working").verb, "Planning");
            s.set_phase(Phase::Reconnecting);
            assert_eq!(s.indicator().expect("working").verb, "Reconnecting");
        }

        /// One turn's calls must not appear under the next one's prompt.
        #[test]
        fn a_new_turn_starts_with_nothing_in_flight() {
            let mut s = working();
            s.set_phase(Phase::Thinking);
            s.start_activity(Activity::running("Read", "a.rs"));
            s.complete("done", Vec::new(), 0);
            assert!(s.running.is_none());
            assert!(s.phase.is_none());

            s.type_char('b');
            s.submit();
            assert!(s.indicator().expect("working").verb != "Read(a.rs)");
        }

        /// A cancelled turn that already did things keeps them. The prompt stays put too:
        /// offering it back would invite redoing work that is on the screen, and some of it
        /// touched the workspace.
        #[test]
        fn cancelling_after_work_keeps_the_record_rather_than_un_sending_it() {
            let mut s = session();
            for c in "do the thing".chars() {
                s.type_char(c);
            }
            let prompt = s.submit().expect("submitted");
            s.start_activity(Activity::running("Write", "a.rs"));
            s.finish_activity(Activity::running("Write", "a.rs").done("3 lines"));

            s.restore(prompt);

            assert!(s.input.is_empty(), "the prompt was offered back");
            assert!(
                s.transcript.iter().any(|e| e.speaker == Speaker::User),
                "the prompt was removed even though work had happened"
            );
            assert!(
                s.transcript.iter().any(|e| e.speaker == Speaker::Tool),
                "the record of the write was thrown away"
            );
            assert_eq!(s.status, Status::Idle);
        }

        /// With nothing done, cancelling still un-sends the whole thing, which is what makes
        /// Escape usable as a change of mind.
        #[test]
        fn cancelling_before_anything_happens_still_un_sends_the_prompt() {
            let mut s = session();
            for c in "never mind".chars() {
                s.type_char(c);
            }
            let prompt = s.submit().expect("submitted");
            s.restore(prompt);

            assert_eq!(s.input, "never mind");
            assert!(s.transcript.is_empty());
        }
    }

    mod replay {
        use super::*;
        use bravebot_agent::Conversation;
        use bravebot_aichat::protocol::Message;
        use std::collections::BTreeMap;

        fn line(text: &str) -> TrailLine {
            TrailLine {
                text: text.to_string(),
                blocked: false,
            }
        }

        fn trails(entries: &[(usize, &str)]) -> BTreeMap<usize, Vec<TrailLine>> {
            let mut map: BTreeMap<usize, Vec<TrailLine>> = BTreeMap::new();
            for (turn, text) in entries {
                map.entry(*turn).or_default().push(line(text));
            }
            map
        }

        fn resumed(messages: Vec<Message>, trails: &BTreeMap<usize, Vec<TrailLine>>) -> Vec<Entry> {
            replayed(
                messages,
                crate::sessions::Recalled {
                    trails: trails.clone(),
                    todos: BTreeMap::new(),
                },
            )
        }

        fn replayed(messages: Vec<Message>, recalled: crate::sessions::Recalled) -> Vec<Entry> {
            let mut conversation = Conversation::new();
            for message in messages {
                conversation.push(message);
            }
            let mut s = session();
            s.replay(&conversation, "a title", &recalled);
            s.transcript
        }

        /// The audit is written beside the record, so what a gate decided two sessions ago is on
        /// disk. Not reading it back is what left Ctrl-T blank over everything before the resume.
        #[test]
        fn a_resumed_turn_shows_the_trail_it_left() {
            let transcript = resumed(
                vec![
                    Message::user("first"),
                    Message::assistant("first reply"),
                    Message::user("second"),
                    Message::assistant("second reply"),
                ],
                &trails(&[(1, "capability: file_read granted"), (2, "action: refused")]),
            );

            let first = transcript
                .iter()
                .find(|entry| entry.text == "first reply")
                .expect("the first reply");
            assert_eq!(first.trail, vec![line("capability: file_read granted")]);

            let second = transcript
                .iter()
                .find(|entry| entry.text == "second reply")
                .expect("the second reply");
            assert_eq!(second.trail, vec![line("action: refused")]);
        }

        /// A turn's trail belongs to the turn, not to each thing it said. Repeating it under
        /// every narration would make one file read look like four.
        #[test]
        fn a_turn_that_spoke_several_times_shows_its_trail_once() {
            let transcript = resumed(
                vec![
                    Message::user("do it"),
                    Message::assistant("looking"),
                    Message::assistant("still looking"),
                    Message::assistant("done"),
                ],
                &trails(&[(1, "capability: file_read granted")]),
            );

            let with_trail: Vec<&str> = transcript
                .iter()
                .filter(|entry| !entry.trail.is_empty())
                .map(|entry| entry.text.as_str())
                .collect();
            assert_eq!(with_trail, vec!["done"], "the trail was repeated");
        }

        /// A turn that was refused before it answered still had gates decide things, and that
        /// record is the one a user most wants. It goes on the prompt, since there is nothing
        /// else of that turn to hang it on.
        #[test]
        fn a_turn_that_never_answered_keeps_its_trail_on_the_prompt() {
            let transcript = resumed(
                vec![Message::user("do the thing")],
                &trails(&[(1, "action: refused")]),
            );

            let prompt = transcript
                .iter()
                .find(|entry| entry.text == "do the thing")
                .expect("the prompt");
            assert_eq!(prompt.trail, vec![line("action: refused")]);
        }

        /// A session resumed with no audit beside it is not an error: it draws the transcript it
        /// has, with nothing under it.
        #[test]
        fn a_session_with_no_audit_replays_without_one() {
            let transcript = resumed(
                vec![Message::user("hello"), Message::assistant("hi")],
                &BTreeMap::new(),
            );
            assert!(transcript.iter().all(|entry| entry.trail.is_empty()));
        }

        /// The plan a turn worked to is beneath it in the scrollback while the session runs, and
        /// was blank under every turn of a resumed one.
        #[test]
        fn a_resumed_turn_shows_the_plan_it_worked_to() {
            use bravebot_core::todo::{Item, List, Status, rows};

            let plan = rows(&List::new(vec![
                Item::new("read the file", Status::Done),
                Item::new("change it", Status::Active),
            ]));
            let transcript = replayed(
                vec![
                    Message::user("first"),
                    Message::assistant("first reply"),
                    Message::user("second"),
                    Message::assistant("second reply"),
                ],
                crate::sessions::Recalled {
                    trails: BTreeMap::new(),
                    todos: BTreeMap::from([(2, plan.clone())]),
                },
            );

            let second = transcript
                .iter()
                .find(|entry| entry.text == "second reply")
                .expect("the second reply");
            assert_eq!(second.todos, plan);

            let first = transcript
                .iter()
                .find(|entry| entry.text == "first reply")
                .expect("the first reply");
            assert!(
                first.todos.is_empty(),
                "one turn's plan appeared under another's work"
            );
        }

        /// The counting has to agree with what `todos_by_turn` wrote, or a plan comes back under
        /// a turn it was never part of.
        #[test]
        fn the_turn_a_plan_is_written_under_is_the_turn_it_comes_back_under() {
            use bravebot_core::todo::{Item, List, Status, rows};

            let plan = rows(&List::new(vec![Item::new("do it", Status::Active)]));

            let mut s = session();
            s.type_char('a');
            s.submit();
            s.complete("first reply", Vec::new(), 0);
            s.type_char('b');
            s.submit();
            s.set_todos(plan.clone());
            s.complete("second reply", Vec::new(), 0);

            let written = s.todos_by_turn();
            assert_eq!(written.keys().copied().collect::<Vec<_>>(), vec![2]);

            let transcript = replayed(
                vec![
                    Message::user("a"),
                    Message::assistant("first reply"),
                    Message::user("b"),
                    Message::assistant("second reply"),
                ],
                crate::sessions::Recalled {
                    trails: BTreeMap::new(),
                    todos: written,
                },
            );
            let second = transcript
                .iter()
                .find(|entry| entry.text == "second reply")
                .expect("the second reply");
            assert_eq!(second.todos, plan);
        }

        /// A transcript that says the model answered and never says it read anything is a poor
        /// account of a turn that spent most of itself reading.
        #[test]
        fn a_resumed_transcript_shows_the_calls_the_turn_made() {
            use bravebot_aichat::protocol::{ToolCallRequest, ToolCallRequestFunction};

            let call = ToolCallRequest {
                id: "call-1".to_string(),
                kind: "function".to_string(),
                function: ToolCallRequestFunction {
                    name: "read_file".to_string(),
                    arguments: r#"{"path":"src/main.rs"}"#.to_string(),
                },
            };
            let transcript = resumed(
                vec![
                    Message::user("what is in main.rs?"),
                    Message::assistant_calling("let me look", vec![call]),
                    Message::assistant("a hello world"),
                ],
                &BTreeMap::new(),
            );

            let call_line = transcript
                .iter()
                .find(|entry| entry.speaker == Speaker::Tool)
                .expect("the call is in the transcript");
            assert_eq!(call_line.text, "Read(src/main.rs)");
            // No outcome is claimed, because the record does not say what came of it.
            assert!(call_line.activity.is_none());
        }

        /// A turn's trail still lands on the last thing the turn said, and a call is a thing the
        /// turn said. Anything else would put the trail above work it covers.
        #[test]
        fn a_trail_lands_after_the_calls_the_turn_made() {
            use bravebot_aichat::protocol::{ToolCallRequest, ToolCallRequestFunction};

            let call = ToolCallRequest {
                id: "call-1".to_string(),
                kind: "function".to_string(),
                function: ToolCallRequestFunction {
                    name: "search".to_string(),
                    arguments: r#"{"pattern":"MAX_STEPS"}"#.to_string(),
                },
            };
            let transcript = resumed(
                vec![
                    Message::user("find it"),
                    Message::assistant_calling(String::new(), vec![call]),
                ],
                &trails(&[(1, "capability: search granted")]),
            );

            let last = transcript.last().expect("something was replayed");
            assert_eq!(last.text, "Search(MAX_STEPS)");
            assert_eq!(last.trail, vec![line("capability: search granted")]);
        }

        /// Starting the counter again at zero understated a resumed session by everything it had
        /// already spent, which is the whole of what the figure is there to report.
        #[test]
        fn a_resumed_session_carries_on_counting_what_it_has_spent() {
            let mut s = session();
            s.restore_spend(4_200, std::collections::BTreeMap::from([(1, 4_200)]));
            assert_eq!(s.tokens, 4_200);

            s.type_char('a');
            s.submit();
            s.complete("reply", Vec::new(), 800);
            assert_eq!(s.tokens, 5_000, "the turn's cost did not add to the total");
        }

        /// A total alone cannot tell an even session from one turn that ran away, and those want
        /// different fixes. The breakdown is what distinguishes them.
        #[test]
        fn each_turn_records_what_it_cost_on_its_own() {
            let mut s = session();

            s.type_char('a');
            s.submit();
            s.complete("first", Vec::new(), 400);

            s.type_char('b');
            s.submit();
            s.complete("second", Vec::new(), 1_600);

            assert_eq!(
                s.spend_by_turn(),
                &std::collections::BTreeMap::from([(1, 400), (2, 1_600)])
            );
            assert_eq!(s.tokens, 2_000, "the breakdown and the total disagreed");
        }

        /// `/compact` happens in the middle of a turn, so its cost belongs to that turn. Charging
        /// it to nothing would leave the breakdown adding up to less than the total.
        #[test]
        fn an_aside_is_charged_to_the_turn_it_interrupted() {
            let mut s = session();

            s.type_char('a');
            s.submit();
            s.complete("reply", Vec::new(), 400);

            s.begin_aside();
            s.end_aside(250);

            assert_eq!(
                s.spend_by_turn(),
                &std::collections::BTreeMap::from([(1, 650)])
            );
            assert_eq!(s.tokens, 650, "the breakdown and the total disagreed");
        }

        /// The whole point of keeping the split: a turn's wall clock alone cannot say whether it was
        /// slow because the model was, or because it stopped and waited for a person.
        #[test]
        fn each_turn_records_where_its_time_went() {
            use bravebot_agent::timing::Timing;
            let mut s = session();

            s.type_char('a');
            s.submit();
            s.complete("first", Vec::new(), 400);
            s.spent_time(Timing {
                // The worker's own wall figure is deliberately ignored: the session has a better
                // one, taken from the moment the prompt was submitted.
                wall_ms: 999_999,
                inference_ms: 300,
                tools_ms: 100,
                stalled_ms: 5_000,
            });

            let first = s
                .timing_by_turn()
                .get(&1)
                .copied()
                .expect("turn 1 recorded");
            assert_eq!(first.inference_ms, 300);
            assert_eq!(first.tools_ms, 100);
            assert_eq!(
                first.stalled_ms, 5_000,
                "the time spent waiting on a person was not kept"
            );
            assert_ne!(
                first.wall_ms, 999_999,
                "the worker's wall figure overwrote the session's own"
            );
        }

        /// `/compact` is a model call made in the middle of a turn, so its wait is that turn's and it
        /// is inference rather than time nobody can account for.
        #[test]
        fn an_aside_charges_its_wait_to_the_turn_it_interrupted() {
            let mut s = session();

            s.type_char('a');
            s.submit();
            s.complete("reply", Vec::new(), 400);

            s.begin_aside();
            s.end_aside(250);

            let turn = s
                .timing_by_turn()
                .get(&1)
                .copied()
                .expect("turn 1 recorded");
            assert_eq!(
                turn.inference_ms, turn.wall_ms,
                "an aside's wait was not counted as time spent on the model"
            );
        }

        /// A turn that failed after ten minutes still spent them, and it is the turn most worth
        /// reading afterwards. A session adding up its own figures must not omit its failures.
        #[test]
        fn a_failed_turn_still_accounts_for_its_wall_clock() {
            let mut s = session();

            s.type_char('a');
            s.submit();
            s.fail("it went wrong");

            assert!(
                s.timing_by_turn().contains_key(&1),
                "a failed turn left no trace of the time it spent"
            );
        }

        /// A record written before timing was kept has a token breakdown and no time breakdown, and
        /// resuming one must restore what there is rather than refusing both.
        #[test]
        fn a_resumed_session_carries_on_from_the_time_it_had_spent() {
            use bravebot_agent::timing::Timing;
            let mut s = session();
            s.restore_timing(std::collections::BTreeMap::from([(
                1,
                Timing {
                    wall_ms: 8_000,
                    inference_ms: 3_000,
                    tools_ms: 1_000,
                    stalled_ms: 4_000,
                },
            )]));

            s.type_char('a');
            s.submit();
            s.complete("reply", Vec::new(), 800);
            s.spent_time(Timing {
                inference_ms: 1_000,
                ..Timing::default()
            });

            let total = s.timing_total();
            assert_eq!(total.inference_ms, 4_000, "the earlier turn was dropped");
            assert_eq!(total.stalled_ms, 4_000);
            assert!(
                total.wall_ms >= 8_000,
                "the resumed turn's wall clock was lost"
            );
        }

        /// Clearing begins a new session, and a new session has spent nothing. A breakdown left
        /// behind would attribute the previous session's cost to this one's turns.
        #[test]
        fn clearing_forgets_what_each_turn_cost() {
            let mut s = session();

            s.type_char('a');
            s.submit();
            s.complete("reply", Vec::new(), 400);

            s.clear();

            assert!(
                s.spend_by_turn().is_empty(),
                "the breakdown outlived the session"
            );
            assert!(
                s.timing_by_turn().is_empty(),
                "the time breakdown outlived the session"
            );
        }

        /// A trail for a turn the conversation does not have must not land on some other turn.
        /// An audit can outlast the record it belongs to, since the two are separate files.
        #[test]
        fn a_trail_for_a_turn_that_is_not_there_lands_nowhere() {
            let transcript = resumed(
                vec![Message::user("only turn"), Message::assistant("only reply")],
                &trails(&[(1, "capability: file_read granted"), (7, "action: refused")]),
            );

            assert!(
                transcript
                    .iter()
                    .all(|entry| entry.trail != vec![line("action: refused")]),
                "a trail from a turn that is not in the transcript was drawn on one that is"
            );
        }
    }

    /// Constructing a session must do no I/O, or every test would read and write the developer's
    /// own history and a second run would see the first run's prompts.
    #[test]
    fn a_plain_session_does_not_persist() {
        let mut s = session();
        for c in "not stored".chars() {
            s.type_char(c);
        }
        s.submit().expect("submitted");

        // In memory for recall, but nothing was written: `persist` is off.
        assert_eq!(s.history.len(), 1);
        assert!(!s.persist, "a plain session was persisting");
    }

    /// The whole of the key: a line goes away and comes back the same. Nothing about it is sent,
    /// so the words have to survive the round trip exactly as they were written.
    #[test]
    fn a_stashed_line_comes_back_as_it_was() {
        let mut s = session();
        for c in "half a thought".chars() {
            s.type_char(c);
        }

        assert!(s.stash(), "nothing was put away");
        assert_eq!(s.input, "", "the line stayed in the box");

        assert!(s.stash(), "nothing came back");
        assert_eq!(s.input, "half a thought");
    }

    /// The caret goes to the end of the line coming back, which is where somebody carries on
    /// typing. It is not where they left it, because the edit it belonged to is over.
    #[test]
    fn the_caret_lands_at_the_end_of_a_line_brought_back() {
        let mut s = session();
        for c in "a thought".chars() {
            s.type_char(c);
        }
        s.move_to_line_start();
        s.stash();
        s.stash();

        assert_eq!(s.caret, s.input.len(), "the caret was not at the end");
    }

    /// One slot, so the second line put away is the one that comes back. A press that quietly
    /// stacked would leave the first line reachable only by pressing again, which is a depth the
    /// key does not advertise and nothing on the screen could report.
    #[test]
    fn stashing_again_replaces_what_was_put_away() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.stash();
        for c in "second".chars() {
            s.type_char(c);
        }
        s.stash();

        s.stash();
        assert_eq!(s.input, "second");
    }

    /// Bringing a line back empties the slot: it is in the box now, and the only copy of it is the
    /// one in front of the user. Left behind, the next press would put a second copy beside a line
    /// they had started editing.
    #[test]
    fn a_line_brought_back_cannot_be_brought_back_again() {
        let mut s = session();
        s.type_char('x');
        s.stash();
        s.stash();
        assert_eq!(s.stashed(), None, "the slot kept a copy");

        // Cleared rather than sent, so the box is empty and a press means "bring one back". There
        // is nothing to bring, and the line the user has since typed is not re-created.
        s.clear_input();
        assert!(!s.stash(), "a line came back twice");
        assert_eq!(s.input, "");
    }

    /// An empty box with nothing put away has nothing to do either way, and says so, so the press
    /// can be told from one that acted.
    #[test]
    fn stashing_an_empty_line_with_nothing_put_away_does_nothing() {
        let mut s = session();
        assert!(!s.stash());
        assert_eq!(s.input, "");
        assert_eq!(s.stashed(), None);
    }

    /// The words travel and the mode does not. Shell mode is a mode of the box rather than part of
    /// the line, so a prompt put away as a prompt comes back into an armed shell as the command the
    /// person is now writing, which is what they asked for by arming it.
    #[test]
    fn the_mode_is_not_stashed_with_the_line() {
        let mut s = session();
        for c in "cargo test".chars() {
            s.type_char(c);
        }
        s.stash();
        assert!(!s.shell, "putting a line away armed shell mode");

        s.type_char('!');
        assert!(s.shell, "shell mode was not armed");
        s.stash();

        assert_eq!(s.input, "cargo test");
        assert!(s.shell, "the line coming back disarmed the mode");
    }

    /// A command put away is text like any other, and the `!` was never part of it. So the mode
    /// stays behind when the line goes, and the words come back as a prompt unless the person has
    /// armed it again themselves.
    #[test]
    fn a_command_comes_back_as_words_and_not_as_a_command() {
        let mut s = session();
        s.type_char('!');
        for c in "rm -rf build".chars() {
            s.type_char(c);
        }
        assert!(s.shell);

        s.stash();
        s.shell = false;
        s.stash();

        assert_eq!(s.input, "rm -rf build");
        assert!(!s.shell, "the line brought the mode back with it");
    }

    /// Allowed mid-turn, like typing and recall: it writes a line and sends nothing, and sending is
    /// the whole of what a running turn refuses. It is also when a person most wants a half-written
    /// thought out of the way, since it is when a better one has just occurred to them.
    #[test]
    fn a_line_can_be_stashed_while_a_turn_runs() {
        let mut s = session();
        s.type_char('a');
        s.submit();
        assert_eq!(s.status, Status::Working);

        for c in "the next thing".chars() {
            s.type_char(c);
        }
        assert!(s.stash(), "nothing was put away mid-turn");
        assert_eq!(s.input, "");

        assert!(s.stash(), "nothing came back mid-turn");
        assert_eq!(s.input, "the next thing");
    }

    /// A marker is text in the line, and what it stands for stays staged while the words are away.
    /// Cleared instead, a line would come back naming a picture that was no longer there, and the
    /// prompt would go with a marker standing over nothing.
    #[test]
    fn what_a_stashed_line_named_is_still_named_when_it_comes_back() {
        let mut s = session();
        for c in "look at ".chars() {
            s.type_char(c);
        }
        s.attach(picture(b"pixels"));
        let line = s.input.clone();

        s.stash();
        assert!(
            s.pasted_named(&s.input).is_empty(),
            "a line that is not in the box still named a picture"
        );

        s.stash();
        assert_eq!(s.input, line);
        assert_eq!(
            s.pasted_named(&s.input).len(),
            1,
            "the picture did not survive the round trip"
        );
    }

    /// A session editing vi's way, with the choice recorded nowhere: `adopt_editing` reads the store
    /// only for a session that persists, so this reads no file and writes none.
    fn vi() -> Session {
        let mut s = Session::new("none");
        s.choose_editing(crate::vim::Editing::Vi);
        s
    }

    /// Every session opens taking typed characters as typed characters. Opening in NORMAL would mean
    /// the first sentence somebody wrote went nowhere, and a box that swallows what is typed into it
    /// is indistinguishable from one that has stopped working.
    #[test]
    fn a_box_that_edits_vis_way_still_opens_taking_letters_as_letters() {
        let mut s = vi();
        for c in "hello".chars() {
            s.type_char(c);
        }
        assert_eq!(s.input, "hello");
        assert_eq!(s.vi_mode(), Some(crate::vim::Mode::Insert));
    }

    /// The box everybody else has is in no mode at all, so nothing can draw one at somebody who never
    /// asked for vi editing, and their Escape goes on discarding the line.
    #[test]
    fn the_ordinary_box_is_in_no_vi_mode_and_cannot_enter_one() {
        let mut s = Session::new("none");
        assert_eq!(s.vi_mode(), None);
        assert!(
            !s.enter_vi_normal(),
            "the ordinary box was put into a vi mode"
        );
        assert_eq!(s.vi_mode(), None);
    }

    /// The whole of what the mode means: a letter is an instruction rather than a letter, and one vi
    /// does not use does nothing at all. Falling through to the line would make NORMAL mode a place
    /// where half the alphabet quietly edits the prompt.
    #[test]
    fn a_letter_typed_in_normal_mode_does_not_reach_the_line() {
        let mut s = vi();
        for c in "hello".chars() {
            s.type_char(c);
        }
        s.enter_vi_normal();

        s.type_char('z');
        s.type_char('q');

        assert_eq!(s.input, "hello", "a letter was typed in NORMAL mode");
    }

    /// The caret in NORMAL mode sits on a character rather than between two, so the position past the
    /// end of the line is not one it can hold. Escape at the end of a line lands on the last character
    /// typed, which is where every vi leaves it and where `x` then has something to delete.
    #[test]
    fn leaving_insert_mode_puts_the_caret_on_a_character() {
        let mut s = vi();
        for c in "hi".chars() {
            s.type_char(c);
        }
        assert_eq!(s.caret, 2);

        s.enter_vi_normal();

        assert_eq!(s.caret, 1, "the caret stayed past the end of the line");
    }

    /// `!` is one of vi's keys in NORMAL mode, not the way shell mode is armed. Reading it as the
    /// marker would arm a mode from a press asking for something else, and there is no way back to a
    /// prompt from a shell armed by accident except deleting past it.
    #[test]
    fn the_shell_marker_is_not_armed_from_normal_mode() {
        let mut s = vi();
        s.enter_vi_normal();

        s.type_char('!');

        assert!(!s.shell, "NORMAL mode armed shell mode");
        assert_eq!(s.input, "", "the marker was typed into the line");
    }

    /// The list of keys is up on a press of `?`, and in NORMAL mode `?` is one of vi's own. A list
    /// that came up here would answer a press that was asking to search backwards.
    #[test]
    fn the_key_list_is_not_opened_from_normal_mode() {
        let mut s = vi();
        s.enter_vi_normal();

        s.type_char('?');

        assert!(!s.shortcuts, "NORMAL mode put the list of keys up");
    }

    /// The six keys that open INSERT mode, each landing the caret where vi lands it. `i` before the
    /// character the caret is on, and `a` after it, is the whole difference between the two.
    #[test]
    fn the_keys_that_open_insert_mode_land_the_caret_where_vi_does() {
        let opened = |key: char, at: usize| {
            let mut s = vi();
            for c in "one two".chars() {
                s.type_char(c);
            }
            s.enter_vi_normal();
            s.caret = 4;
            s.type_char(key);
            assert_eq!(
                s.vi_mode(),
                Some(crate::vim::Mode::Insert),
                "{key} did not open INSERT mode"
            );
            assert_eq!(s.caret, at, "{key} left the caret in the wrong place");
        };

        opened('i', 4);
        opened('a', 5);
        opened('I', 0);
        opened('A', 7);
    }

    /// `o` opens a line below and `O` one above, with the caret on the new empty line either way.
    /// Landing it on the line that was already there would leave somebody typing into the sentence
    /// they had just asked to write beneath.
    #[test]
    fn opening_a_line_leaves_the_caret_on_the_new_one() {
        let mut s = vi();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.enter_vi_normal();
        s.type_char('o');
        for c in "second".chars() {
            s.type_char(c);
        }
        assert_eq!(s.input, "first\nsecond");

        let mut s = vi();
        for c in "second".chars() {
            s.type_char(c);
        }
        s.enter_vi_normal();
        s.type_char('O');
        for c in "first".chars() {
            s.type_char(c);
        }
        assert_eq!(s.input, "first\nsecond");
    }

    /// The choice is about the person and the mode is about the moment. Somebody who has just turned
    /// vi editing on is not expecting the next letter to be an instruction, and NORMAL is not a state
    /// the ordinary box has at all.
    #[test]
    fn choosing_a_style_of_editing_leaves_the_box_taking_letters() {
        let mut s = vi();
        s.enter_vi_normal();
        assert_eq!(s.vi_mode(), Some(crate::vim::Mode::Normal));

        s.choose_editing(crate::vim::Editing::Vi);

        assert_eq!(s.vi_mode(), Some(crate::vim::Mode::Insert));
    }

    /// A settings file answers for somebody who has never chosen, and a word naming no style leaves
    /// the box everybody has: a session that gave somebody vi editing on a typo would be one where
    /// the letters stopped working for a reason they cannot see.
    #[test]
    fn a_configured_style_is_adopted_and_an_unknown_word_is_not() {
        let mut s = Session::new("none");
        s.adopt_editing(Some("vim"));
        assert_eq!(s.editing(), crate::vim::Editing::Vi);

        let mut s = Session::new("none");
        s.adopt_editing(Some("modal"));
        assert_eq!(s.editing(), crate::vim::Editing::Ordinary);

        let mut s = Session::new("none");
        s.adopt_editing(None);
        assert_eq!(s.editing(), crate::vim::Editing::Ordinary);
    }

    /// A session in NORMAL mode over `line`, with the caret where `at` says.
    fn normal(line: &str, at: usize) -> Session {
        let mut s = vi();
        for c in line.chars() {
            s.type_char(c);
        }
        s.enter_vi_normal();
        s.caret = at;
        s
    }

    /// Where a run of presses in NORMAL mode leaves the caret.
    fn after(line: &str, at: usize, keys: &str) -> usize {
        let mut s = normal(line, at);
        for c in keys.chars() {
            s.type_char(c);
        }
        s.caret
    }

    /// The keys under the fingers, and Space among them because that is what vi does with it. `j` and
    /// `k` are not here: they spell Up and Down, which reach the prompt history past the ends of the
    /// input, so the key handler answers them and pins them.
    #[test]
    fn the_character_motions_move_one_character() {
        assert_eq!(after("hello", 2, "h"), 1);
        assert_eq!(after("hello", 2, "l"), 3);
        assert_eq!(after("hello", 2, " "), 3);
    }

    /// `w` lands on the first character of the next word and `b` on the first of this one or the
    /// previous, which is not what the word keys under Ctrl do: those land after the word they
    /// crossed. Both are wanted, and vi's is what an instruction spelled `w` has to mean.
    #[test]
    fn the_word_motions_land_where_vi_lands() {
        assert_eq!(after("one two three", 0, "w"), 4);
        assert_eq!(after("one two three", 0, "ww"), 8);
        assert_eq!(after("one two three", 4, "b"), 0);
        // `e` reaches the end of this word, and from an end the end of the next.
        assert_eq!(after("one two three", 0, "e"), 2);
        assert_eq!(after("one two three", 2, "e"), 6);
    }

    /// The ends of the line, and the first character that is not a blank. `$` lands on the last
    /// character rather than past it, since the column after the line holds nothing for an
    /// instruction to act on.
    #[test]
    fn the_line_motions_reach_the_ends_and_the_first_word() {
        assert_eq!(after("  indented", 5, "0"), 0);
        assert_eq!(after("  indented", 5, "^"), 2);
        assert_eq!(after("hello", 0, "$"), 4);
    }

    /// `gg` and `G` reach the whole input rather than the line, which is what makes them worth having
    /// in a box that holds a paragraph. `G` lands at the start of the last line, as vi does.
    #[test]
    fn the_input_motions_reach_the_first_and_last_line() {
        assert_eq!(after("one\ntwo\nthree", 9, "gg"), 0);
        assert_eq!(after("one\ntwo\nthree", 1, "G"), 8);
    }

    /// `g` alone means nothing, and a pair that means nothing must not hold the wait open: one stray
    /// press would otherwise swallow every letter after it until something happened to match.
    #[test]
    fn a_pair_that_means_nothing_ends_the_wait_rather_than_holding_it() {
        let mut s = normal("one\ntwo", 5);
        s.type_char('g');
        s.type_char('x');
        assert_eq!(s.caret, 5, "the abandoned pair moved the caret");

        // The next press is read on its own rather than as a third key of the pair.
        s.type_char('0');
        assert_eq!(s.caret, 4);
    }

    /// `f` and `t` search forwards, `F` and `T` back, and the short pair stop one character before
    /// the target. A character that is not on the line leaves the caret alone.
    #[test]
    fn the_jumps_to_a_character_land_on_it_or_just_short_of_it() {
        assert_eq!(after("one two three", 0, "ft"), 4);
        assert_eq!(after("one two three", 0, "tt"), 3);
        assert_eq!(after("one two three", 12, "Ft"), 8);
        assert_eq!(after("one two three", 12, "Tt"), 9);
        assert_eq!(after("one two three", 0, "fz"), 0);
    }

    /// The jump is over the line the caret is on, not the whole input. These keys are for reaching a
    /// bracket in front of you, and one that crossed a newline would land off the row being read.
    #[test]
    fn a_jump_to_a_character_stays_on_its_own_line() {
        assert_eq!(after("one\ntwo", 0, "fw"), 0);
    }

    /// `;` and `,` mean nothing on their own: they say "that again", forwards and then back. Without
    /// them every repeat is the whole pair typed out, which is the thing these keys exist to save.
    #[test]
    fn the_repeat_keys_do_the_last_jump_again_and_then_the_other_way() {
        assert_eq!(after("a-b-c-d", 0, "f-"), 1);
        assert_eq!(after("a-b-c-d", 0, "f-;"), 3);
        assert_eq!(after("a-b-c-d", 0, "f-;;"), 5);
        assert_eq!(after("a-b-c-d", 0, "f-;;,"), 3);
    }

    /// With no jump made, the keys that repeat one have nothing to repeat and do nothing rather than
    /// guessing at a character.
    #[test]
    fn a_repeat_with_nothing_to_repeat_does_nothing() {
        assert_eq!(after("a-b-c-d", 2, ";"), 2);
        assert_eq!(after("a-b-c-d", 2, ","), 2);
    }

    /// A marker stands for one thing, so a motion crosses it whole and cannot come to rest inside
    /// one: a caret between two halves of a picture is in a place the person cannot see, and the next
    /// instruction would act there. Every motion goes through the same caret methods the arrows use,
    /// which is what makes this hold for all of them rather than for the ones somebody remembered.
    #[test]
    fn a_motion_crosses_a_marker_whole() {
        /// A line with a marker in the middle of it, in NORMAL mode with the caret at `at`, and where
        /// the marker begins and ends. Words on both sides, so the caret has somewhere to be on the far
        /// side and what is measured is the crossing rather than the end of the line.
        fn staged(at: impl Fn(usize, usize) -> usize) -> (Session, usize, usize) {
            let mut s = vi();
            for c in "look at ".chars() {
                s.type_char(c);
            }
            s.attach(picture(b"pixels"));
            for c in " and say".chars() {
                s.type_char(c);
            }
            let opens = s.input.find('[').expect("the marker is in the line");
            let closes = s.input.find(']').expect("the marker is in the line") + 1;
            s.enter_vi_normal();
            s.caret = at(opens, closes);
            (s, opens, closes)
        }

        // Every motion that could reach the marker, from the side it would reach it from. One press
        // crosses the whole of it and there is no press that lands within it.
        let crossings = [
            ("l", true),
            ("w", true),
            ("e", true),
            ("f]", true),
            ("$", true),
            ("h", false),
            ("b", false),
            ("F[", false),
            ("0", false),
        ];
        for (keys, forwards) in crossings {
            let (mut s, opens, closes) =
                staged(|opens, closes| if forwards { opens } else { closes });
            for c in keys.chars() {
                s.type_char(c);
            }
            assert!(
                s.caret <= opens || s.caret >= closes,
                "{keys} left the caret at {} inside the marker {opens}..{closes}",
                s.caret
            );
        }
    }

    /// `k` and `j` are Up and Down, and `/` is the chord that searches the prompts already sent. What
    /// those reach is not the line, so they are named for the key handler to answer rather than moving
    /// the caret from here: at the ends of the input they walk the prompt history and then scroll.
    #[test]
    fn the_letters_that_spell_other_keys_are_named_rather_than_acted_on() {
        let s = normal("one\ntwo", 0);
        assert_eq!(s.vi_spells('k'), Some(Spelled::Up));
        assert_eq!(s.vi_spells('j'), Some(Spelled::Down));
        assert_eq!(s.vi_spells('/'), Some(Spelled::SearchPrompts));
        assert_eq!(s.vi_spells('l'), None);

        // Nothing at all in INSERT mode, where every one of them is a character to type.
        let mut typing = vi();
        assert_eq!(typing.vi_spells('j'), None);
        assert_eq!(typing.vi_spells('/'), None);
        typing.type_char('/');
        assert_eq!(typing.input, "/");
    }

    /// While a key waits for its character, every press is that character: `f/` jumps to a slash
    /// rather than opening a search, and `fj` to a `j` rather than walking a row.
    #[test]
    fn a_key_waiting_for_its_character_claims_the_letters_that_spell_other_keys() {
        let mut s = normal("a/b", 0);
        s.type_char('f');
        assert_eq!(s.vi_spells('/'), None, "the wait did not claim the press");
        assert_eq!(s.vi_spells('j'), None);

        s.type_char('/');
        assert_eq!(s.caret, 1, "the jump did not happen");
    }

    /// What a run of presses in NORMAL mode leaves the line as.
    fn edited(line: &str, at: usize, keys: &str) -> String {
        let mut s = normal(line, at);
        for c in keys.chars() {
            s.type_char(c);
        }
        s.input
    }

    /// One operator over one set of extents, which is what makes these one idea rather than a binding
    /// each: the letter says what happens and the rest says where.
    #[test]
    fn the_delete_operator_takes_the_stretch_a_motion_names() {
        assert_eq!(edited("one two three", 0, "dw"), "two three");
        assert_eq!(edited("one two three", 4, "de"), "one  three");
        assert_eq!(edited("one two three", 4, "db"), "two three");
        assert_eq!(edited("one two three", 4, "d$"), "one ");
        assert_eq!(edited("one two three", 0, "dfo"), " three");
        assert_eq!(edited("one two three", 0, "dto"), "o three");
    }

    /// `x` takes the character under the caret and `dd` the whole line, newline and all: a line taken
    /// out has to close the gap it left rather than leaving a blank one where it was.
    #[test]
    fn the_character_and_the_line_are_extents_of_their_own() {
        assert_eq!(edited("hello", 1, "x"), "hllo");
        assert_eq!(edited("one\ntwo\nthree", 4, "dd"), "one\nthree");
        assert_eq!(edited("one\ntwo", 5, "dd"), "one");
        assert_eq!(edited("only", 1, "dd"), "");
        assert_eq!(edited("one two", 3, "D"), "one");
    }

    /// `c` is `d` and then INSERT mode, which is the whole of the difference: it takes the same stretch
    /// and leaves the person typing where it was.
    #[test]
    fn the_change_operator_takes_the_stretch_and_starts_typing() {
        let mut s = normal("one two", 0);
        s.type_char('c');
        s.type_char('w');
        // `cw` on a character that is not a blank is `ce`: the space after the word stays, where `dw`
        // would take it. Vi's own special case, because a word replaced and run into the next one is
        // never what was wanted.
        assert_eq!(s.input, " two");
        assert_eq!(s.vi_mode(), Some(crate::vim::Mode::Insert));
        s.type_char('x');
        assert_eq!(s.input, "x two", "the box was not taking letters");

        // `s` and `S` are the same operator over a character and a line.
        assert_eq!(edited("hello", 0, "sx"), "xello");
        assert_eq!(edited("one\ntwo", 4, "Sx"), "one\nx");
        assert_eq!(edited("one two", 4, "Cx"), "one x");
    }

    /// `cw` on a character that is not a blank is `ce`, leaving the space that `dw` would take, and on
    /// a blank it is `dw` again. Vi's own special case, kept because the alternative is useless: a word
    /// replaced and run into the next one is never what somebody meant, and typing the space back every
    /// time is what the key would otherwise cost.
    #[test]
    fn changing_a_word_leaves_the_space_after_it() {
        assert_eq!(edited("one two", 0, "cwX"), "X two");
        assert_eq!(edited("one two", 0, "dw"), "two");
        // On a blank there is no word to change, so it takes the blanks as `dw` does.
        assert_eq!(edited("one  two", 3, "cwX"), "oneXtwo");
    }

    /// `y` reads without writing, which is why there is nothing for undo to put back after one. The
    /// line is untouched and the register holds what was under the motion.
    #[test]
    fn the_yank_operator_leaves_the_line_alone() {
        let mut s = normal("one two", 0);
        s.type_char('y');
        s.type_char('w');
        assert_eq!(s.input, "one two", "yanking changed the line");

        // Which is proved by putting it back: `p` is the only way to see what the register holds.
        s.type_char('p');
        assert_eq!(s.input, "oone ne two");
    }

    /// A yanked line goes back as a line and a yanked word beside the caret, which is what the register
    /// remembers besides the text. Without it, `yy` then `p` splices a sentence into the middle of
    /// another one.
    #[test]
    fn a_yanked_line_comes_back_as_a_line() {
        assert_eq!(edited("one\ntwo", 0, "yyp"), "one\none\ntwo");
        assert_eq!(edited("one\ntwo", 0, "yyP"), "one\none\ntwo");
        assert_eq!(edited("one\ntwo", 4, "yyP"), "one\ntwo\ntwo");
    }

    /// `P` puts it before the character the caret is on and `p` after it, which is where vi puts them:
    /// the caret sits on a character rather than between two, so no single position means "here" for
    /// both keys.
    #[test]
    fn the_register_goes_back_on_either_side_of_the_caret() {
        assert_eq!(edited("ab", 0, "ylp"), "aab");
        assert_eq!(edited("ab", 1, "ylP"), "abb");
    }

    /// With nothing yanked there is nothing to put back, and the key does nothing rather than guessing
    /// at what to insert.
    #[test]
    fn putting_back_an_empty_register_does_nothing() {
        assert_eq!(edited("hello", 0, "p"), "hello");
        assert_eq!(edited("hello", 0, "P"), "hello");
    }

    /// `>>` and `<<` move the line by a fixed number of spaces rather than a tab, because the box draws
    /// what it holds and a tab's width is the terminal's opinion. Dedenting a line at the margin has
    /// nothing to take and leaves it there.
    #[test]
    fn the_line_shifts_by_spaces_and_stops_at_the_margin() {
        assert_eq!(edited("one", 0, ">>"), "  one");
        assert_eq!(edited("one", 0, ">>>>"), "    one");
        assert_eq!(edited("    one", 4, "<<"), "  one");
        assert_eq!(edited("one", 0, "<<"), "one");
    }

    /// `J` makes two lines one, with a single space where the newline was: two sentences run together
    /// with no gap is not what somebody joining lines wants, and the blanks the next line was indented
    /// with are part of a shape it no longer has.
    #[test]
    fn joining_puts_one_space_where_the_newline_was() {
        assert_eq!(edited("one\ntwo", 0, "J"), "one two");
        assert_eq!(edited("one\n    two", 0, "J"), "one two");
        assert_eq!(edited("only", 0, "J"), "only", "there was no line to join");
    }

    /// `u` puts back what the last change took. The failure worth ruling out is the one that loses a
    /// paragraph: every operator that writes records the line first, in the one place they all pass
    /// through, so none of them can be the one that forgot.
    #[test]
    fn undo_puts_back_what_a_change_took() {
        for keys in ["dw", "dd", "D", "x", "cw", "J", ">>", "p"] {
            let mut s = normal("one two\nthree", 0);
            // So that `p` has something to put back, and every case starts from the same line.
            s.register = Some(Yanked {
                text: "x".to_string(),
                lines: false,
            });
            for c in keys.chars() {
                s.type_char(c);
            }
            assert_ne!(s.input, "one two\nthree", "{keys} changed nothing to undo");

            s.enter_vi_normal();
            s.type_char('u');
            assert_eq!(s.input, "one two\nthree", "{keys} could not be undone");
        }
    }

    /// Undo with nothing to undo does nothing, and a yank is not a change: there is nothing to put back
    /// after one, so `u` after it must not reach past it to an earlier change.
    #[test]
    fn there_is_nothing_to_undo_after_a_yank_or_before_a_change() {
        assert_eq!(edited("hello", 0, "u"), "hello");

        let mut s = normal("one two", 0);
        s.type_char('d');
        s.type_char('w');
        s.type_char('y');
        s.type_char('w');
        s.type_char('u');
        assert_eq!(s.input, "one two", "the yank got in the way of the undo");
    }

    /// `.` repeats the instruction rather than what it produced, which is the whole reason to have it:
    /// the change happens again at the caret, wherever that now is.
    #[test]
    fn the_repeat_key_does_the_last_change_again_at_the_caret() {
        assert_eq!(edited("one two three", 0, "dw."), "three");
        assert_eq!(edited("aaa", 0, "x."), "a");
    }

    /// A text object is a thing the line is made of rather than a distance, which is the whole reason
    /// for them: `ci(` is what somebody means when they want the arguments replaced, and the
    /// alternative is counting characters to a bracket they can see perfectly well.
    ///
    /// Every case here was measured against vim rather than reasoned about.
    #[test]
    fn a_word_is_a_text_object_with_and_without_the_blanks_around_it() {
        assert_eq!(edited("one two three", 1, "diw"), " two three");
        assert_eq!(edited("one two three", 1, "daw"), "two three");
        // A run of blanks is a run too, so the caret is always in something.
        assert_eq!(edited("one  two", 3, "diw"), "onetwo");
        // The blanks come from before the word where there are none after it, or `daw` on the last word
        // of a line would be `diw`.
        assert_eq!(edited("one two three", 4, "daw"), "one three");
        assert_eq!(edited("one two three", 9, "daw"), "one two");
    }

    /// `W` folds punctuation into the word, so a path or a flag is one thing. `w` does not, which is what
    /// makes it useful on `src/main.rs`: the slashes are words of their own there.
    #[test]
    fn a_bigword_is_everything_that_is_not_a_blank() {
        assert_eq!(edited("a/b c/d", 1, "diW"), " c/d");
        assert_eq!(edited("a/b c/d", 1, "daW"), "c/d");
        assert_eq!(edited("a/b c/d", 1, "diw"), "ab c/d");
    }

    /// Quotes and brackets, inside and around. Either half of a pair names it, since `di(` and `di)` are
    /// the same request and nobody wants to think about which they typed.
    #[test]
    fn a_pair_of_delimiters_is_a_text_object() {
        assert_eq!(edited("say \"hello there\" ok", 6, "di\""), "say \"\" ok");
        assert_eq!(edited("say \"hello there\" ok", 6, "da\""), "say ok");
        assert_eq!(edited("say 'a quote' ok", 6, "di'"), "say '' ok");
        assert_eq!(edited("say 'a quote' ok", 6, "da'"), "say ok");
        assert_eq!(edited("call(a, b) ok", 5, "di("), "call() ok");
        assert_eq!(edited("call(a, b) ok", 5, "da("), "call ok");
        assert_eq!(edited("call(a, b) ok", 5, "di)"), "call() ok");
        assert_eq!(edited("x[1] ok", 2, "di["), "x[] ok");
        assert_eq!(edited("a{b}c", 2, "di{"), "a{}c");
    }

    /// The pair the caret is inside, or else the next one along the line. The second half is what makes
    /// `ci(` work with the caret on the name in front of the bracket, which is where it usually is.
    #[test]
    fn a_pair_is_the_one_around_the_caret_or_the_next_one_along() {
        assert_eq!(edited("call(a, b) ok", 0, "di("), "call() ok");
        assert_eq!(
            edited("aaaaaaaa call(x) ok", 0, "di("),
            "aaaaaaaa call() ok"
        );
        // Nested, where the pair is the one that encloses rather than whichever opened first.
        assert_eq!(edited("f(g(x))", 4, "di("), "f(g())");
        // Nothing to find, so the line is left alone rather than something nearby being taken.
        assert_eq!(edited("call(x) then here", 14, "di("), "call(x) then here");
    }

    /// A pair with the caret on one of its own delimiters is that pair, not the next one: the caret is on
    /// the thing being named.
    #[test]
    fn a_pair_named_from_its_own_delimiter_is_that_pair() {
        assert_eq!(edited("call(a, b) ok", 4, "di("), "call() ok");
        assert_eq!(edited("(a) then (b)", 4, "di("), "(a) then ()");
    }

    /// A text object composes with every operator, which is the point of it being an extent rather than a
    /// binding: `y` and `c` take the same stretch `d` does.
    #[test]
    fn a_text_object_works_with_every_operator() {
        assert_eq!(edited("one two three", 0, "cawX"), "Xtwo three");
        assert_eq!(edited("one two", 0, "yiwp"), "oonene two");
    }

    /// A key that names no kind of thing ends the wait rather than holding it open, on the same footing
    /// as every other pair that means nothing.
    #[test]
    fn a_pair_naming_no_kind_of_object_does_nothing() {
        assert_eq!(edited("one two", 0, "diz"), "one two");
        assert_eq!(edited("one two", 0, "dizw"), "one two");
    }

    /// A marker is spelled with brackets and a digit, so a text object naming brackets could reach
    /// inside one. It takes the whole marker or none of it, as every operator does: half a marker
    /// stands for nothing, and here the danger is that the object machinery searches the line's
    /// characters rather than walking the caret's own positions.
    #[test]
    fn a_text_object_over_a_marker_takes_it_whole_or_not_at_all() {
        for keys in ["di[", "da[", "diw", "daw", "diW", "daW"] {
            let mut s = vi();
            for c in "look at ".chars() {
                s.type_char(c);
            }
            s.attach(picture(b"pixels"));
            for c in " and say".chars() {
                s.type_char(c);
            }
            let before = s.input.clone();
            s.enter_vi_normal();
            s.caret = before.find('[').expect("the marker is in the line");

            for c in keys.chars() {
                s.type_char(c);
            }

            let whole = !s.input.contains('[') && !s.input.contains(']');
            assert!(
                whole || s.input == before,
                "{keys} left half a marker: {:?}",
                s.input
            );
        }
    }

    /// A marker is one thing, so an operator takes the whole of it or none: half a marker stands for
    /// nothing, and text that still reads as an attachment over something no longer attached is the
    /// failure this rules out. It holds because a stretch is measured between positions the caret could
    /// rest at, and none of those is inside a marker.
    #[test]
    fn an_operator_takes_a_marker_whole() {
        for keys in ["x", "dw", "d$", "D", "dl"] {
            let mut s = vi();
            for c in "look at ".chars() {
                s.type_char(c);
            }
            s.attach(picture(b"pixels"));
            for c in " and say".chars() {
                s.type_char(c);
            }
            let opens = s.input.find('[').expect("the marker is in the line");
            s.enter_vi_normal();
            s.caret = opens;

            for c in keys.chars() {
                s.type_char(c);
            }

            assert!(
                !s.input.contains('[') && !s.input.contains(']'),
                "{keys} left half a marker: {:?}",
                s.input
            );
        }
    }

    /// A deleted marker takes the attachment off, which is what a marker is for: it is the only thing
    /// the person can see to delete, and a picture still attached to a line that no longer names it
    /// would go with the prompt unseen.
    #[test]
    fn an_operator_that_takes_a_marker_takes_the_attachment_with_it() {
        let mut s = vi();
        s.attach(picture(b"pixels"));
        assert_eq!(s.pasted_named(&s.input).len(), 1);
        s.enter_vi_normal();
        s.caret = 0;

        s.type_char('x');

        assert!(
            s.pasted_named(&s.input).is_empty(),
            "the picture is still named by a line that has no marker"
        );
    }
}
