//! Session state, kept separate from rendering so it can be tested without a terminal.
//!
//! A session is a sequence of independent turns. It holds transcript and input state for
//! display; it does **not** hold a policy. Each turn constructs its own, which is what
//! stops routing from one turn leaking into the next as untrusted content accumulates.

use crate::audit::TrailLine;
use bravebot_agent::report::{Activity, Landing, Phase, Shown};
use bravebot_core::event::Event;
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
    /// What the last frame laid the transcript out to.
    pub laid: Laid,
    /// Confinement in force, reported so the user knows what they have.
    pub confinement: String,
    /// How many turns have been submitted, which picks the indicator's word.
    pub turns: usize,
    /// Tokens spent across the whole session.
    pub tokens: u64,
    /// How large the last request was, and the budget it is compacted at.
    ///
    /// `None` until a request has been measured, which is the honest reading: nothing has been
    /// counted yet, and drawing a gauge at zero would claim it had.
    ///
    /// Not the same figure as [`Session::tokens`], which adds every round of every turn together
    /// and so says what the session has cost. This says how full the context is now.
    occupancy: Option<(u64, u64)>,
    /// Prompts already sent, for recall with the arrow keys.
    pub history: crate::history::History,
    /// What the mouse is sweeping over, or what it last swept over.
    ///
    /// Kept after the button comes up, so a user can see what they copied rather than watching
    /// it vanish at the moment it is taken.
    pub selection: Option<crate::select::Selection>,
    /// How much the last copy took, until the next thing happens.
    pub copied: Option<usize>,
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
    /// The tool call in flight, if one is.
    ///
    /// Also in the transcript, where it stays. Kept here as well because the indicator needs
    /// to name it, and scanning back through the transcript for the tail would be a worse way
    /// to answer a question the session already knows the answer to.
    pub running: Option<Activity>,
    /// Prompts typed and sent while a turn was running, in the order they were typed.
    ///
    /// Not in the transcript: they have not happened. They are drawn under the box as waiting,
    /// and each moves into the transcript at the moment its own turn begins.
    pub queued: Vec<Queued>,
    /// The reply the model is writing right now, as far as it has got.
    ///
    /// Not in the transcript, because it is not a thing that happened yet: it is drawn at the
    /// tail and replaced by the entry the round produces. Keeping it apart is what makes that
    /// handover free of a duplicate, and it is why a session written to disk holds finished
    /// turns rather than a half-finished sentence.
    pub streaming: String,
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
            status: Status::Idle,
            show_trail: false,
            scroll: 0,
            scroller: None,
            laid: Laid::default(),
            confinement: confinement.into(),
            turns: 0,
            tokens: 0,
            occupancy: None,
            history: crate::history::History::new(),
            selection: None,
            copied: None,
            cleared_by_interrupt: false,
            image_on_clipboard: false,
            written: 0,
            todos: Vec::new(),
            phase: None,
            running: None,
            queued: Vec::new(),
            streaming: String::new(),
            answers: Vec::new(),
            pasted: Vec::new(),
            sent_pasted: Vec::new(),
            pasted_text: Vec::new(),
            persist: false,
            said: Vec::new(),
            started: None,
            model: None,
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
    pub fn restore_spend(&mut self, tokens: u64) {
        self.tokens = tokens;
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
    pub fn clear(&mut self) {
        self.transcript.clear();
        self.turns = 0;
        self.tokens = 0;
        self.occupancy = None;
        self.written = 0;
        self.todos.clear();
        self.phase = None;
        self.running = None;
        self.started = None;
        self.scroll = 0;
        self.selection = None;
        self.copied = None;
        // A standing condition is worth saying once per session, and this is now a new one: the
        // reason a skill was left out applies to the next turn as much as it did to the last.
        self.said.clear();
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
        self.streaming.push_str(text);
        self.scroll = 0;
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
        if text.trim().is_empty() {
            return;
        }
        self.transcript.push(Entry::assistant(text, Vec::new()));
    }

    /// Show a tool call that has begun.
    pub fn start_activity(&mut self, activity: Activity) {
        self.running = Some(activity.clone());
        self.transcript.push(Entry::tool(activity));
    }

    /// Record where the last call's result went.
    pub fn landed(&mut self, landing: Landing) {
        if let Some(entry) = self.transcript.last_mut()
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
        match self.transcript.last_mut() {
            Some(entry) if entry.speaker == Speaker::Tool && entry.shown.is_none() => {
                entry.shown = Some(shown);
            }
            _ => {
                let mut entry = Entry::system("");
                entry.shown = Some(shown);
                self.transcript.push(entry);
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
        match self.transcript.last_mut() {
            Some(entry) if entry.speaker == Speaker::Tool && Self::still_running(entry) => {
                *entry = Entry::tool(activity);
            }
            _ => self.transcript.push(Entry::tool(activity)),
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
    fn set_input(&mut self, line: impl Into<String>) {
        self.input = line.into();
        self.caret = self.input.len();
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
        if self.status == Status::Working {
            return Offered::Nothing;
        }
        // A command line is neither a slash command nor a sentence with a file reference in it.
        // `/usr/bin/env` and an address with an `@` in it are ordinary arguments here, and
        // completing either would rewrite the line under someone typing a path.
        if self.shell {
            return Offered::Nothing;
        }
        // The list of keys takes the place of what is offered, since the two answer the same space
        // and only one of them was asked for.
        if self.shortcuts {
            return Offered::Shortcuts;
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

        self.note(format!("resumed session: {title}"));

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
        let marker = format!(
            "[Pasted text #{} +{} lines]",
            self.attachments_made,
            lines_in(&text)
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
    /// Called where the turn is built and nowhere else. The session keeps the folded line
    /// throughout, so the box, the transcript, the history and a cancelled turn coming back all
    /// say what the user was looking at, and only the thing that talks to the model is given the
    /// words.
    pub fn unfolded(&self, line: &str) -> String {
        let mut line = line.to_string();
        for pasted in &self.pasted_text {
            line = line.replace(&pasted.marker, &pasted.text);
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
            self.set_input(prompt);
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
            // behind would arm the next thing typed as a command.
            self.shell = false;
            // And so does the list of keys: Escape takes down whatever is up, and on an empty line
            // the list is the only thing there is to take down.
            self.shortcuts = false;
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
        self.history.push(line.clone());
        if self.persist {
            crate::store::append_history(&line);
        }
        self.transcript.push(Entry::shell(line.clone()));
        self.scroll = 0;
        Some(line)
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
            self.transcript.push(Entry::system("no output"));
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
    pub fn submit(&mut self) -> Option<String> {
        if self.status != Status::Idle {
            return None;
        }
        let prompt = self.input.trim().to_string();
        if prompt.is_empty() {
            return None;
        }
        // Settled here, from the line as it was sent. Everything still named goes; a marker the
        // user deleted is an attachment they took off, and it goes nowhere. Pictures settle the
        // same way and at the same moment, since a marker rubbed out means the same thing whether
        // the thing behind it was dropped or pasted.
        let taken = self.take_line(&prompt);
        // Recorded here rather than in `begin_turn`, because a queued prompt was recorded when it
        // was queued: from the person's side that is when they sent it.
        self.history.push(prompt.clone());
        if self.persist {
            crate::store::append_history(&prompt);
        }
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
        let prompt = self.input.trim().to_string();
        if prompt.is_empty() {
            return false;
        }
        let (attached, pasted) = self.take_line(&prompt);
        self.history.push(prompt.clone());
        if self.persist {
            crate::store::append_history(&prompt);
        }
        self.queued.push(Queued {
            prompt,
            attached,
            pasted,
        });
        self.scroll = 0;
        true
    }

    /// Begin the turn for the prompt queued longest ago, if the session is free to start one.
    pub fn send_queued(&mut self) -> Option<String> {
        if self.status != Status::Idle || self.queued.is_empty() {
            return None;
        }
        let next = self.queued.remove(0);
        Some(self.begin_turn(next.prompt, (next.attached, next.pasted)))
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
        // The box is about to be rewritten, so the things standing over it that belong to the line
        // it held go, exactly as they do for anything else that writes a whole line.
        self.history.leave();
        self.shortcuts = false;
        self.completion = 0;

        let mut lines = Vec::new();
        let mut attached = Vec::new();
        let mut pasted = Vec::new();
        for waiting in self.queued.drain(..) {
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
        self.transcript
            .push(Entry::assistant(reply, trail).with_todos(todos));
        self.status = Status::Idle;
        self.scroll = 0;
        self.started = None;
        self.phase = None;
        self.running = None;
        self.streaming.clear();
        // Accumulated across the session: the figure answers "what has this cost me", which is
        // about the session rather than the last turn.
        self.tokens += tokens;
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
        self.started = None;
        self.phase = None;
        self.running = None;
        self.streaming.clear();
    }

    /// Record how full the context is, against the budget it is compacted at.
    pub fn measured(&mut self, used: u64, budget: u64) {
        self.occupancy = Some((used, budget));
    }

    /// How full the context is, as a percentage, or `None` where nothing has been measured.
    ///
    /// Capped at a hundred rather than allowed past it. The budget is a guess at a window nobody
    /// reports, so a request larger than it is a session that will be compacted next round, not a
    /// context that is a hundred and forty per cent full.
    /// Zero reads as "not measured" rather than as an empty context. No request costs nothing, so
    /// the figure only ever arrives as zero when there has not been one to count: before the
    /// first turn, and after a compaction has shortened the conversation underneath it.
    pub fn fullness(&self) -> Option<u64> {
        let (used, budget) = self.occupancy?;
        (used > 0 && budget > 0).then(|| (used * 100 / budget).min(100))
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
    pub fn end_aside(&mut self, tokens: u64) {
        self.status = Status::Idle;
        self.started = None;
        self.phase = None;
        self.running = None;
        self.tokens += tokens;
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

    pub fn quit(&mut self) {
        self.status = Status::Quitting;
    }

    pub fn is_quitting(&self) -> bool {
        self.status == Status::Quitting
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
    fn a_folded_paste_is_put_back_before_the_turn_is_built() {
        let mut s = session();
        for c in "what is ".chars() {
            s.type_char(c);
        }
        s.paste_text("one\ntwo\nthree\n");

        let prompt = s.submit().expect("submitted");
        assert_eq!(prompt, "what is [Pasted text #1 +3 lines]");
        assert_eq!(s.unfolded(&prompt), "what is one\ntwo\nthree\n");
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
        assert_eq!(s.unfolded(&prompt), "never mind");
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

    /// A prompt recalled out of the history comes back with its marker in it. A marker naming
    /// nothing would send the placeholder in place of everything the user pasted, and there is
    /// nothing on the screen that would tell them it had.
    #[test]
    fn a_recalled_prompt_still_names_what_was_pasted_into_it() {
        let mut s = session();
        s.paste_text("one\ntwo\nthree\n");
        let first = s.submit().expect("submitted");
        s.complete("three lines", Vec::new(), 0);

        s.set_input(first);
        let again = s.submit().expect("submitted");
        assert_eq!(s.unfolded(&again), "one\ntwo\nthree\n");
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
    #[test]
    fn editing_a_recalled_prompt_stops_browsing_history() {
        let mut s = session();
        s.history.push("an older prompt".to_string());
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
        s.measured(25_000, 100_000);
        assert_eq!(s.fullness(), Some(25));
    }

    /// The budget is a guess at a window nobody reports, so a request larger than it is a session
    /// about to be compacted rather than a context a hundred and forty per cent full.
    #[test]
    fn a_request_past_the_budget_reads_as_full_rather_than_more_than_full() {
        let mut s = Session::new("none");
        s.measured(140_000, 100_000);
        assert_eq!(s.fullness(), Some(100));
    }

    /// After a compaction nothing has been counted for the shortened conversation, and the old
    /// figure describes an exchange that is no longer being sent. Better to say nothing until the
    /// next turn counts it than to show a percentage that is no longer about anything.
    #[test]
    fn a_context_measured_at_nothing_is_a_context_nobody_has_measured() {
        let mut s = Session::new("none");
        s.measured(0, 100_000);
        assert_eq!(s.fullness(), None);
    }

    /// A new session's context is empty, so the gauge from the old one would be describing a
    /// conversation that no longer exists.
    #[test]
    fn clearing_a_session_forgets_how_full_the_old_one_was() {
        let mut s = Session::new("none");
        s.measured(90_000, 100_000);
        s.clear();
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
            s.restore_spend(4_200);
            assert_eq!(s.tokens, 4_200);

            s.type_char('a');
            s.submit();
            s.complete("reply", Vec::new(), 800);
            assert_eq!(s.tokens, 5_000, "the turn's cost did not add to the total");
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
}
