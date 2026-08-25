//! Session state, kept separate from rendering so it can be tested without a terminal.
//!
//! A session is a sequence of independent turns. It holds transcript and input state for
//! display; it does **not** hold a policy. Each turn constructs its own, which is what
//! stops routing from one turn leaking into the next as untrusted content accumulates.

use crate::audit::TrailLine;
use bua_agent::report::{Activity, Phase};
use bua_core::event::Event;
use std::time::{Duration, Instant};

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
    pub todos: Vec<bua_core::todo::Row>,
    /// The call this entry describes, for a [`Speaker::Tool`] entry.
    ///
    /// Carries the note and the hunks separately from `text` so the interface can style them
    /// without parsing anything back out of a formatted line.
    pub activity: Option<Activity>,
}

impl Entry {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::User,
            text: text.into(),
            trail: Vec::new(),
            todos: Vec::new(),
            activity: None,
        }
    }

    pub fn assistant(text: impl Into<String>, trail: Vec<TrailLine>) -> Self {
        Self {
            speaker: Speaker::Assistant,
            text: text.into(),
            trail,
            todos: Vec::new(),
            activity: None,
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::System,
            text: text.into(),
            trail: Vec::new(),
            todos: Vec::new(),
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
            activity: None,
        }
    }

    /// Attach the task list the turn finished with.
    pub fn with_todos(mut self, todos: Vec<bua_core::todo::Row>) -> Self {
        self.todos = todos;
        self
    }
}

/// What the session is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting for input.
    Idle,
    /// A turn is in flight. Input is refused so a second turn cannot start mid-flight and
    /// share the first one's state.
    Working,
    /// The user asked to leave.
    Quitting,
}

/// Everything the interface needs to draw itself.
#[derive(Debug)]
pub struct Session {
    pub transcript: Vec<Entry>,
    pub input: String,
    pub status: Status,
    /// Whether the audit trail is shown alongside replies.
    pub show_trail: bool,
    /// Scroll offset from the bottom, in lines.
    pub scroll: u16,
    /// Confinement in force, reported so the user knows what they have.
    pub confinement: String,
    /// How many turns have been submitted, which picks the indicator's word.
    pub turns: usize,
    /// Tokens spent across the whole session.
    pub tokens: u64,
    /// Prompts already sent, for recall with the arrow keys.
    pub history: crate::history::History,
    /// What the mouse is sweeping over, or what it last swept over.
    ///
    /// Kept after the button comes up, so a user can see what they copied rather than watching
    /// it vanish at the moment it is taken.
    pub selection: Option<crate::select::Selection>,
    /// How much the last copy took, until the next thing happens.
    pub copied: Option<usize>,
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
    pub todos: Vec<bua_core::todo::Row>,
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
}

impl Session {
    pub fn new(confinement: impl Into<String>) -> Self {
        Self {
            transcript: Vec::new(),
            input: String::new(),
            status: Status::Idle,
            show_trail: false,
            scroll: 0,
            confinement: confinement.into(),
            turns: 0,
            tokens: 0,
            history: crate::history::History::new(),
            selection: None,
            copied: None,
            written: 0,
            todos: Vec::new(),
            phase: None,
            running: None,
            persist: false,
            said: Vec::new(),
            started: None,
        }
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
        self.persist = true;
        self
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
        if let Some(running) = &self.running {
            return Some(running.line());
        }
        if let Some(row) = self
            .todos
            .iter()
            .find(|row| row.status == bua_core::todo::Status::Active)
        {
            return Some(row.content.clone());
        }
        self.phase.map(|phase| phase.word().to_string())
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
    pub fn set_todos(&mut self, rows: Vec<bua_core::todo::Row>) {
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

    /// The task list each turn finished with, by turn number, for writing the session down.
    ///
    /// Read back off the transcript rather than kept in a second place, because the transcript is
    /// already where a finished turn's list lives: `complete` moves it there so the scrollback
    /// shows what each turn set out to do. Turn numbers are counted the way `replay` counts them,
    /// so a list written under turn three comes back under turn three.
    pub fn todos_by_turn(&self) -> std::collections::BTreeMap<usize, Vec<bua_core::todo::Row>> {
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
    }

    /// Record what the model said on its way to the next tool call.
    ///
    /// Empty text is dropped here rather than by the turn, which cannot look at it to decide.
    /// This side may: the text has been released, and a blank line in a transcript is a
    /// presentation question.
    pub fn narrate(&mut self, text: impl Into<String>) {
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
        // Editing a recalled prompt makes it the working line rather than a view of history,
        // so the position indicator goes away as soon as a key is pressed.
        self.history.leave();
        self.input.push(c);
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
        conversation: &bua_agent::Conversation,
        title: &str,
        recalled: &crate::sessions::Recalled,
    ) {
        use bua_agent::conversation::Said;

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
        self.input
            .push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
    }

    pub fn backspace(&mut self) {
        self.history.leave();
        self.input.pop();
    }

    /// Put a submitted prompt back for editing after its turn was cancelled.
    ///
    /// The text returns to the box so a user who changed their mind can adjust it rather than
    /// retype it, which is the whole point of cancelling rather than waiting.
    pub fn restore(&mut self, prompt: impl Into<String>) {
        self.status = Status::Idle;
        self.started = None;
        self.phase = None;
        self.running = None;
        self.scroll = 0;

        // Nothing was recorded after the prompt, so there is nothing to have second thoughts
        // about: the turn can be un-sent whole.
        if !matches!(self.transcript.last(), Some(entry) if entry.speaker == Speaker::User) {
            // Otherwise the turn visibly did things, some of which touched the workspace.
            // Putting the prompt back would offer to redo work that is on the screen, and
            // removing what happened would hide it, so both stay and the stop is recorded.
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
            self.input = prompt.into();
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
            self.input.clear();
        }
    }

    /// Show the previous prompt, stepping further back on each call.
    pub fn recall_older(&mut self) {
        if self.status != Status::Idle {
            return;
        }
        if let Some(prompt) = self.history.older(&self.input) {
            self.input = prompt;
        }
    }

    /// Step forward through recalled prompts, back to the line being typed.
    pub fn recall_newer(&mut self) {
        if self.status != Status::Idle {
            return;
        }
        if let Some(prompt) = self.history.newer() {
            self.input = prompt;
        }
    }

    /// Take the current input as a prompt, if there is one.
    ///
    /// Clears the field and records the prompt in the transcript, so the display reflects
    /// the submission even before a reply arrives.
    /// Take the input as a local command rather than as a turn.
    ///
    /// A sibling of [`Session::submit`] that deliberately stops short of the two things that
    /// make an input a turn: the status does not go to `Working` and the turn count does not
    /// move. Everything else is the same, because a command is still something the user typed
    /// and still belongs in the transcript and in the history the arrow keys walk.
    pub fn submit_command(&mut self) -> Option<String> {
        if self.status != Status::Idle {
            return None;
        }
        let command = self.input.trim().to_string();
        if command.is_empty() {
            return None;
        }
        self.input.clear();
        self.history.push(command.clone());
        if self.persist {
            crate::store::append_history(&command);
        }
        self.transcript.push(Entry::user(command.clone()));
        self.scroll = 0;
        Some(command)
    }

    pub fn submit(&mut self) -> Option<String> {
        if self.status != Status::Idle {
            return None;
        }
        let typed = self.input.trim();
        // `//` is how a prompt that must begin with a slash gets sent, now that a single slash
        // means a command. One slash is removed, which is the one that did the escaping.
        let prompt = match typed.strip_prefix('/') {
            Some(rest) if rest.starts_with('/') => rest.to_string(),
            _ => typed.to_string(),
        };
        if prompt.is_empty() {
            return None;
        }
        self.input.clear();
        self.history.push(prompt.clone());
        if self.persist {
            crate::store::append_history(&prompt);
        }
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
        Some(prompt)
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
    }

    pub fn note(&mut self, message: impl Into<String>) {
        self.transcript.push(Entry::system(message));
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

    pub fn toggle_trail(&mut self) {
        self.show_trail = !self.show_trail;
    }

    pub fn quit(&mut self) {
        self.status = Status::Quitting;
    }

    pub fn is_quitting(&self) -> bool {
        self.status == Status::Quitting
    }

    pub fn scroll_up(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_add(lines);
    }

    pub fn scroll_down(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_sub(lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bua_core::label::Label;

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

    /// A command is answered locally. If it flipped the session to Working it would sit there
    /// with a spinner waiting for a turn that was never started.
    #[test]
    fn a_command_does_not_start_a_turn() {
        let mut s = session();
        for c in "/skills".chars() {
            s.type_char(c);
        }
        let command = s.submit_command().expect("submitted");

        assert_eq!(command, "/skills");
        assert_eq!(s.status, Status::Idle, "the session went to work");
        assert_eq!(s.turns, 0, "a command was counted as a turn");
        assert!(s.input.is_empty());
    }

    /// A command is still something the user typed, so walking back through the input should
    /// find it. Leaving it out would make the arrow keys skip over what was just done.
    #[test]
    fn a_command_is_still_recalled_by_the_arrow_keys() {
        let mut s = session();
        for c in "/skills".chars() {
            s.type_char(c);
        }
        s.submit_command().expect("submitted");

        s.recall_older();
        assert_eq!(s.input, "/skills");
    }

    /// A single slash now means a command, so without an escape a prompt beginning with one
    /// would be unreachable. `//` sends it, minus the slash that did the escaping.
    #[test]
    fn a_prompt_can_still_begin_with_a_slash() {
        let mut s = session();
        for c in "//usr/bin is on PATH".chars() {
            s.type_char(c);
        }
        let prompt = s.submit().expect("submitted");

        assert_eq!(prompt, "/usr/bin is on PATH");
        assert_eq!(
            s.status,
            Status::Working,
            "an escaped prompt is still a turn"
        );
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

    /// A pasted paragraph keeps its lines: it was written with them, and the box draws them.
    /// Line endings from anywhere land as the same thing, so text copied out of a document
    /// written on Windows does not arrive with the returns still in it.
    #[test]
    fn a_paste_keeps_its_lines_however_they_were_written() {
        let mut s = session();
        s.paste("first\r\nsecond\rthird\nfourth");
        assert_eq!(s.input, "first\nsecond\nthird\nfourth");
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
        s.restore(&prompt);

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
                capability: bua_core::capability::Capability::FileRead,
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
        // `submit` takes the text, so put something back the way only a bug could.
        s.input.push_str("mid-turn");

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

    /// Recall is refused mid-turn, like the other input methods: the box is not the user's to
    /// edit while a turn owns it.
    #[test]
    fn recall_is_refused_while_a_turn_is_running() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.submit().expect("submitted");

        s.recall_older();
        assert!(s.input.is_empty(), "history was recalled mid-turn");
    }
    mod todos {
        use super::*;
        use bua_core::todo::{Item, List, Status, rows};

        fn list(entries: &[(&str, Status)]) -> Vec<bua_core::todo::Row> {
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

        /// The active task names the indicator, which is what makes the line say what is
        /// happening rather than a generic word.
        #[test]
        fn the_active_task_names_the_indicator() {
            let mut s = working();
            s.set_todos(list(&[
                ("Escape cancels a turn", Status::Done),
                ("Add prompt history", Status::Active),
            ]));

            assert_eq!(s.indicator().expect("working").verb, "Add prompt history");
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
        use bua_agent::report::{Activity, Phase};

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

        /// The indicator names the call in flight, which is the most specific thing true at
        /// that moment.
        #[test]
        fn a_running_call_names_the_indicator() {
            let mut s = working();
            s.set_phase(Phase::Thinking);
            s.start_activity(Activity::running("Search", "MAX_STEPS"));
            assert_eq!(s.indicator().expect("working").verb, "Search(MAX_STEPS)");
        }

        /// And gives the name back when it ends, rather than leaving a finished call on the
        /// line as though it were still going.
        #[test]
        fn a_finished_call_stops_naming_the_indicator() {
            let mut s = working();
            s.set_phase(Phase::Thinking);
            s.start_activity(Activity::running("Search", "MAX_STEPS"));
            s.finish_activity(Activity::running("Search", "MAX_STEPS").done("2 matches"));
            assert_eq!(s.indicator().expect("working").verb, "Thinking");
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

        /// A task the model marked in progress beats the phase word: it says what the work is,
        /// not merely what the turn is waiting on.
        #[test]
        fn an_active_task_beats_the_phase_word() {
            let mut s = working();
            s.set_phase(Phase::Thinking);
            s.set_todos(bua_core::todo::rows(&bua_core::todo::List::new(vec![
                bua_core::todo::Item::new("Add prompt history", bua_core::todo::Status::Active),
            ])));
            assert_eq!(s.indicator().expect("working").verb, "Add prompt history");
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
        use bua_agent::Conversation;
        use bua_aichat::protocol::Message;
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
            use bua_core::todo::{Item, List, Status, rows};

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
            use bua_core::todo::{Item, List, Status, rows};

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
            use bua_aichat::protocol::{ToolCallRequest, ToolCallRequestFunction};

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
            use bua_aichat::protocol::{ToolCallRequest, ToolCallRequestFunction};

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
}
