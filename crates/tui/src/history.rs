//! Recalling earlier prompts.
//!
//! Every submitted prompt is kept, and Up walks backwards through them the way a shell does.
//! Retyping a long question because it needed one word changed is the kind of friction that makes
//! an interface tiring.
//!
//! Browsing is a mode rather than an edit: while it is active the box shows a stored prompt and
//! reports the position, and leaving the mode restores whatever was being typed before. That way
//! pressing Up out of curiosity cannot destroy a half-written line.
//!
//! Walking back one at a time is no way to reach the hundredth prompt, so each entry also carries
//! when it was sent and which workspace it was sent from. Both are for
//! [`crate::history_search`], which is the other way in: a list a person reads and narrows, where
//! an age says which of two similar prompts is the one they mean and the workspace says whether a
//! prompt belongs to what they are doing now.

/// One prompt as it was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What was typed, newlines and all.
    pub prompt: String,
    /// When it was sent, in seconds since the epoch.
    ///
    /// `None` for an entry stored before times were kept. Read as "no age to show" rather than as
    /// the epoch, which would date every old prompt to 1970.
    pub at: Option<u64>,
    /// The workspace it was sent from.
    ///
    /// `None` for an entry stored before that was kept, and for one sent from nowhere in
    /// particular. Such an entry belongs to no project and so is never what a narrowed search
    /// answers with, but it is still there under the wider one.
    pub project: Option<String>,
}

impl Entry {
    /// A prompt sent now, from `project`.
    pub fn sent(prompt: impl Into<String>, project: Option<String>) -> Self {
        Self {
            prompt: prompt.into(),
            at: Some(now()),
            project,
        }
    }

    /// A prompt read back from a file that stored nothing else about it.
    pub fn recalled(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            at: None,
            project: None,
        }
    }

    /// The first line, which is what a one-row list can show of a paragraph.
    pub fn opening(&self) -> &str {
        self.prompt.lines().next().unwrap_or("")
    }

    /// How many lines the prompt runs to.
    pub fn lines(&self) -> usize {
        self.prompt.lines().count().max(1)
    }
}

/// Seconds since the epoch, or zero on a clock that cannot say.
///
/// Zero rather than a failure: a prompt is still worth storing on a machine whose clock is wrong,
/// and an age nobody can compute is a missing column rather than a reason to lose the prompt.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Prompts already sent, and where the user is looking.
#[derive(Debug, Default)]
pub struct History {
    /// Oldest first, so the newest is at the end.
    entries: Vec<Entry>,
    /// How far back the user has walked. `None` means they are editing, not browsing.
    ///
    /// Counted from the newest entry: 1 is the most recent prompt. Stored as a distance rather
    /// than an index so appending an entry cannot silently move what is being viewed.
    back: Option<usize>,
    /// What was in the input box before browsing started, to restore on the way out.
    stashed: String,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start from prompts stored by earlier sessions, oldest first.
    pub fn from_entries(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            back: None,
            stashed: String::new(),
        }
    }

    /// Every stored prompt, oldest first, for writing back to disk and for searching.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Record a submitted prompt, sent now from `project`.
    ///
    /// Consecutive duplicates are collapsed: sending the same thing twice is usually a retry, and
    /// two identical entries make walking back slower without adding anything. The kept entry is
    /// the older one, since the prompt is the same and the first time it was asked is when the
    /// question was new.
    pub fn push(&mut self, prompt: impl Into<String>, project: Option<String>) -> Option<&Entry> {
        let entry = Entry::sent(prompt, project);
        self.leave();
        if self
            .entries
            .last()
            .is_some_and(|last| last.prompt == entry.prompt)
        {
            return None;
        }
        self.entries.push(entry);
        self.entries.last()
    }

    /// Remove the newest entry, for a prompt whose turn was cancelled.
    ///
    /// The text goes back into the input box, so keeping it in history too would offer the user
    /// the same line from two places.
    pub fn pop(&mut self) -> Option<Entry> {
        self.leave();
        self.entries.pop()
    }

    /// How many prompts are stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the user is currently looking at a stored prompt.
    pub fn is_browsing(&self) -> bool {
        self.back.is_some()
    }

    /// The position to show, as `(index, total)`, counting oldest first.
    ///
    /// `None` when not browsing. The index is the entry's ordinal rather than its distance back,
    /// because "History 78/83" reads as a place in a list.
    pub fn position(&self) -> Option<(usize, usize)> {
        let back = self.back?;
        Some((self.entries.len() + 1 - back, self.entries.len()))
    }

    /// Step one prompt further back, returning what to show.
    ///
    /// `current` is what is in the input box now, kept so it can be restored on the way out.
    /// Returns `None` at the oldest entry, leaving the view where it is rather than wrapping:
    /// wrapping to the newest would look like the key had stopped working.
    pub fn older(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }

        let back = match self.back {
            None => {
                self.stashed = current.to_string();
                1
            }
            Some(back) if back < self.entries.len() => back + 1,
            // Already at the oldest.
            Some(_) => return None,
        };

        self.back = Some(back);
        self.entries
            .get(self.entries.len() - back)
            .map(|entry| entry.prompt.clone())
    }

    /// Step one prompt forward, returning what to show.
    ///
    /// Stepping forward from the newest entry leaves browsing and restores the line that was
    /// being typed, which is what makes Up safe to press speculatively.
    pub fn newer(&mut self) -> Option<String> {
        match self.back {
            None => None,
            Some(1) => {
                self.back = None;
                Some(std::mem::take(&mut self.stashed))
            }
            Some(back) => {
                self.back = Some(back - 1);
                self.entries
                    .get(self.entries.len() - (back - 1))
                    .map(|entry| entry.prompt.clone())
            }
        }
    }

    /// Stop browsing without changing the input.
    pub fn leave(&mut self) {
        self.back = None;
        self.stashed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(prompts: &[&str]) -> History {
        let mut history = History::new();
        for prompt in prompts {
            history.push(*prompt, None);
        }
        history
    }

    #[test]
    fn a_new_history_is_empty_and_not_browsing() {
        let history = History::new();
        assert!(history.is_empty());
        assert!(!history.is_browsing());
        assert_eq!(history.position(), None);
    }

    /// Up with nothing stored must do nothing rather than clearing the line.
    #[test]
    fn an_empty_history_has_nothing_to_recall() {
        let mut history = History::new();
        assert_eq!(history.older("typing"), None);
        assert!(!history.is_browsing());
    }

    #[test]
    fn up_recalls_the_most_recent_prompt_first() {
        let mut history = with(&["first", "second", "third"]);
        assert_eq!(history.older("").as_deref(), Some("third"));
    }

    #[test]
    fn up_keeps_walking_backwards() {
        let mut history = with(&["first", "second", "third"]);
        assert_eq!(history.older("").as_deref(), Some("third"));
        assert_eq!(history.older("").as_deref(), Some("second"));
        assert_eq!(history.older("").as_deref(), Some("first"));
    }

    /// Wrapping round to the newest would look like the key had stopped working, so the view
    /// stays put at the oldest entry.
    #[test]
    fn up_stops_at_the_oldest_entry() {
        let mut history = with(&["only"]);
        assert_eq!(history.older("").as_deref(), Some("only"));
        assert_eq!(history.older(""), None);
        assert_eq!(history.position(), Some((1, 1)));
    }

    #[test]
    fn down_walks_forwards_again() {
        let mut history = with(&["first", "second", "third"]);
        history.older("");
        history.older("");
        assert_eq!(history.newer().as_deref(), Some("third"));
    }

    /// The reason Up is safe to press speculatively: whatever was being typed comes back.
    #[test]
    fn leaving_the_newest_entry_restores_the_typed_line() {
        let mut history = with(&["stored"]);
        assert_eq!(history.older("half typed").as_deref(), Some("stored"));
        assert_eq!(history.newer().as_deref(), Some("half typed"));
        assert!(!history.is_browsing());
    }

    #[test]
    fn down_does_nothing_when_not_browsing() {
        let mut history = with(&["stored"]);
        assert_eq!(history.newer(), None);
    }

    /// The position is what the interface shows, so it must match the screenshot's reading: an
    /// ordinal in a list, oldest first.
    #[test]
    fn the_position_counts_from_the_oldest() {
        let mut history = History::new();
        for n in 1..=83 {
            history.push(format!("prompt {n}"), None);
        }

        // One press shows the newest, which is the 83rd of 83.
        history.older("");
        assert_eq!(history.position(), Some((83, 83)));

        // Walking back to the 78th, as in the screenshot.
        for _ in 0..5 {
            history.older("");
        }
        assert_eq!(history.position(), Some((78, 83)));
    }

    /// Sending the same prompt twice is usually a retry, and a duplicate only makes walking back
    /// slower.
    #[test]
    fn consecutive_duplicates_are_collapsed() {
        let history = with(&["same", "same", "other", "same"]);
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn submitting_leaves_browsing() {
        let mut history = with(&["first", "second"]);
        history.older("");
        assert!(history.is_browsing());
        history.push("third", None);
        assert!(!history.is_browsing(), "still browsing after a submission");
    }

    /// A cancelled prompt goes back in the input box, so it must leave history too rather than
    /// being offered from two places.
    #[test]
    fn popping_removes_the_newest_entry() {
        let mut history = with(&["first", "second"]);
        assert_eq!(
            history.pop().map(|entry| entry.prompt).as_deref(),
            Some("second")
        );
        assert_eq!(history.len(), 1);
        assert_eq!(history.older("").as_deref(), Some("first"));
    }

    #[test]
    fn popping_an_empty_history_is_harmless() {
        let mut history = History::new();
        assert_eq!(history.pop(), None);
    }

    /// Browsing then submitting a new prompt must not corrupt the position, which is why the
    /// distance is stored rather than an index.
    #[test]
    fn appending_while_browsing_does_not_shift_the_view() {
        let mut history = with(&["a", "b"]);
        history.older("");
        history.push("c", None);

        // No longer browsing, and a fresh walk back sees the new entry first.
        assert!(!history.is_browsing());
        assert_eq!(history.older("").as_deref(), Some("c"));
        assert_eq!(history.position(), Some((3, 3)));
    }
}
