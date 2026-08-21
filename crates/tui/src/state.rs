//! Session state, kept separate from rendering so it can be tested without a terminal.
//!
//! A session is a sequence of independent turns. It holds transcript and input state for
//! display; it does **not** hold a policy. Each turn constructs its own, which is what
//! stops routing from one turn leaking into the next as untrusted content accumulates.

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
}

/// One entry in the transcript.
#[derive(Debug, Clone)]
pub struct Entry {
    pub speaker: Speaker,
    pub text: String,
    /// Gate events recorded while producing this entry, shown when the trail is visible.
    pub trail: Vec<Event>,
}

impl Entry {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::User,
            text: text.into(),
            trail: Vec::new(),
        }
    }

    pub fn assistant(text: impl Into<String>, trail: Vec<Event>) -> Self {
        Self {
            speaker: Speaker::Assistant,
            text: text.into(),
            trail,
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::System,
            text: text.into(),
            trail: Vec::new(),
        }
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
            started: None,
        }
    }

    /// How long the turn in flight has been running, or zero when idle.
    pub fn elapsed(&self) -> Duration {
        self.started.map(|t| t.elapsed()).unwrap_or_default()
    }

    /// The indicator to show, or `None` when no turn is running.
    pub fn indicator(&self) -> Option<crate::indicator::Indicator> {
        (self.status == Status::Working).then(|| {
            crate::indicator::Indicator::new(
                self.turns.saturating_sub(1),
                self.elapsed(),
                self.tokens,
            )
        })
    }

    /// Accept a typed character. Ignored while a turn is running, so input cannot be
    /// interleaved with a turn in flight.
    pub fn type_char(&mut self, c: char) {
        if self.status == Status::Idle {
            // Editing a recalled prompt makes it the working line rather than a view of history,
            // so the position indicator goes away as soon as a key is pressed.
            self.history.leave();
            self.input.push(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.status == Status::Idle {
            self.history.leave();
            self.input.pop();
        }
    }

    /// Put a submitted prompt back for editing after its turn was cancelled.
    ///
    /// The text returns to the box so a user who changed their mind can adjust it rather than
    /// retype it, which is the whole point of cancelling rather than waiting.
    pub fn restore(&mut self, prompt: impl Into<String>) {
        self.status = Status::Idle;
        self.started = None;
        // Popped because the text is going back into the box: offering it from history as well
        // would present the same line from two places.
        self.history.pop();
        self.input = prompt.into();
        // The submitted prompt is still in the transcript, so it is removed: the turn produced
        // nothing, and leaving it would read as a question that went unanswered.
        if matches!(self.transcript.last(), Some(entry) if entry.speaker == Speaker::User) {
            self.transcript.pop();
        }
        self.scroll = 0;
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
    pub fn submit(&mut self) -> Option<String> {
        if self.status != Status::Idle {
            return None;
        }
        let prompt = self.input.trim().to_string();
        if prompt.is_empty() {
            return None;
        }
        self.input.clear();
        self.history.push(prompt.clone());
        self.transcript.push(Entry::user(prompt.clone()));
        self.status = Status::Working;
        self.scroll = 0;
        self.turns += 1;
        self.started = Some(Instant::now());
        Some(prompt)
    }

    /// Record a completed turn, and what it cost.
    pub fn complete(&mut self, reply: impl Into<String>, trail: Vec<Event>, tokens: u64) {
        self.transcript.push(Entry::assistant(reply, trail));
        self.status = Status::Idle;
        self.scroll = 0;
        self.started = None;
        // Accumulated across the session: the figure answers "what has this cost me", which is
        // about the session rather than the last turn.
        self.tokens += tokens;
    }

    /// Record a failure. The turn is over either way, so the session returns to idle.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.transcript.push(Entry::system(message));
        self.status = Status::Idle;
        self.scroll = 0;
        self.started = None;
    }

    pub fn note(&mut self, message: impl Into<String>) {
        self.transcript.push(Entry::system(message));
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

    #[test]
    fn typing_accumulates_input() {
        let mut s = session();
        s.type_char('h');
        s.type_char('i');
        assert_eq!(s.input, "hi");
        s.backspace();
        assert_eq!(s.input, "h");
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

    /// A second turn must not start while one is in flight, since each turn owns its own
    /// policy and interleaving them would blur that boundary.
    #[test]
    fn input_is_refused_while_a_turn_is_running() {
        let mut s = session();
        for c in "first".chars() {
            s.type_char(c);
        }
        s.submit();
        assert_eq!(s.status, Status::Working);

        s.type_char('x');
        assert!(s.input.is_empty(), "typing was accepted mid-turn");
        assert!(s.submit().is_none(), "a second turn was allowed to start");
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
}
