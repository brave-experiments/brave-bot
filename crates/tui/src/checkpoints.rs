//! What a turn changed, kept so it can be put back.
//!
//! One checkpoint per turn, holding the prior contents of every file that turn wrote over. A turn
//! that changed nothing has no checkpoint, because there is nothing to return to.
//!
//! # Not the session record
//!
//! These live in their own tree, beside the records rather than inside them. The record is read
//! back into a later turn's context on a resume, and what is kept here is the contents of files
//! this agent changed, which may be files nobody vouched for. Nothing here is ever read into a
//! turn: it is written on the way past and read only to restore. Deleting the whole tree leaves
//! every session still resumable.
//!
//! # Whole contents, not differences
//!
//! Applying a difference means locating the passage it belongs to, and locating a passage means
//! comparing text. On a file nobody vouched for that comparison is a decision taken from bytes an
//! attacker may have written, which is what an edit is refused for. Whole contents are carried and
//! handed to a write with nothing compared along the way. Growth is answered by the bound below.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The tree these live under, beside `sessions` rather than inside it.
const CHECKPOINTS: &str = "checkpoints";

/// Checkpoints kept for one session.
///
/// A file's contents are far bulkier than a line of transcript, so a long session in a large
/// repository would otherwise grow without limit. The oldest go first, because a rewind reaches
/// for something recent: a turn that went wrong is noticed within a few turns or not at all.
const MAX_CHECKPOINTS: usize = 50;

/// The shortest first sentence worth taking as the whole summary.
///
/// A reply very often opens with "Done!" or "Sure." and a row reading "Done" says less than the
/// file name it replaced, so a stop this early is passed over for the next one.
const MIN_SUMMARY_CHARS: usize = 24;

/// How much of the model's account of a turn a row will carry.
///
/// A reply is prose and can run for paragraphs; a row is one line beside two other columns.
const MAX_SUMMARY_CHARS: usize = 72;

/// The largest file worth keeping a copy of.
///
/// Past this the path is recorded with nothing behind it, so the list still says the file was
/// changed and a restore refuses it rather than silently putting back a truncation.
const MAX_FILE_BYTES: usize = 1_000_000;

/// One file as it stood before a turn wrote over it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Captured {
    /// Workspace-relative, as the tool named it.
    pub path: String,
    /// What the file held, or `None` where the path held no file at all.
    ///
    /// The absence is the point: restoring a creation means removing the file, and an empty
    /// string would put back something that was never there.
    pub prior: Option<String>,
    /// Set where the file was too large to copy, so a restore refuses rather than truncating.
    #[serde(default)]
    pub too_large: bool,
    /// What was done to it: `created`, `replaced` or `edited`, in the driver's own word.
    #[serde(default)]
    pub verb: String,
    /// Lines added and removed, from the same diff the person reviewing the write saw.
    #[serde(default)]
    pub added: usize,
    #[serde(default)]
    pub removed: usize,
}

impl Captured {
    /// What was done to this one file, as a person reads it.
    ///
    /// A creation has no lines removed and every line added, so counting them says nothing a
    /// person wants: the word and the name are the whole story.
    pub fn describe(&self) -> String {
        if self.verb == "created" {
            return bravebot_i18n::t!(checkpoint_created, path = &self.path);
        }
        bravebot_i18n::t!(
            checkpoint_changed,
            path = &self.path,
            added = self.added,
            removed = self.removed
        )
    }
}

/// Everything one turn changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// This checkpoint's own place in the session, counting from one.
    ///
    /// Not the turn. A turn that changed no file leaves no checkpoint, so turns skip and these do
    /// not, and a person reading a list should not have to wonder where 3 went. It also stays the
    /// same for the life of the session, which is what a restore can be asked for by name.
    #[serde(default)]
    pub number: usize,
    /// The turn this came from. Kept because it says where in the conversation this was, and a
    /// restore has to put the conversation back too.
    pub turn: usize,
    /// Seconds since the epoch, for saying how long ago it was.
    pub made: u64,
    /// The model's own account of what the turn did, cut to a line.
    ///
    /// **Untrusted.** Model output, free to describe a change as something gentler than it was, so
    /// it is drawn inside a margin the renderer paints and never counted as a fact about the
    /// change. The files and counts beside it are the driver's own and are what actually happened.
    #[serde(default)]
    pub did: Option<String>,
    /// The files this turn wrote over, in the order it wrote them.
    pub files: Vec<Captured>,
}

impl Checkpoint {
    /// How it reads in a list, and whether those words can be trusted.
    ///
    /// The model's account of the turn where there is one, which reads best and is untrusted: the
    /// caller marks it. Otherwise the driver's own record of the files, which is always true and
    /// never needs marking.
    pub fn summary(&self) -> (String, bool) {
        match &self.did {
            Some(did) => (did.clone(), true),
            None => (self.what_the_files_say(), false),
        }
    }

    /// The driver's own record of what changed, for a row with nothing better to say.
    fn what_the_files_say(&self) -> String {
        match self.files.split_first() {
            Some((only, [])) => only.describe(),
            Some((first, rest)) => bravebot_i18n::t!(
                checkpoint_and_others,
                first = first.describe(),
                others = rest.len()
            ),
            None => bravebot_i18n::t!(checkpoint_unnamed, number = self.number),
        }
    }
}

/// One row of the list, with the trusted parts kept apart from the untrusted one.
///
/// Split here rather than formatted into a line, so the renderer can draw the margin around
/// exactly the words that need it. A formatted string would leave the interface guessing which
/// half of it the model wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// This checkpoint's own number in the session, which does not change as more arrive.
    pub number: String,
    /// What the checkpoint is. Marked when `untrusted`.
    pub summary: String,
    /// Whether the model wrote those words.
    pub untrusted: bool,
    /// Which checkpoint this is and how long ago. The driver's own arithmetic.
    pub when: String,
}

/// The list a person reads, newest first.
pub fn rows(checkpoints: &[Checkpoint]) -> Vec<Row> {
    checkpoints
        .iter()
        .map(|checkpoint| {
            let (summary, untrusted) = checkpoint.summary();
            Row {
                number: format!("{}.", checkpoint.number),
                summary,
                untrusted,
                when: bravebot_i18n::t!(
                    checkpoints_number_and_age,
                    number = checkpoint.number,
                    age = crate::sessions::how_long_ago(checkpoint.made)
                ),
            }
        })
        .collect()
}

/// The model's account of a turn, cut to one line for a list.
///
/// The first sentence, or the first words where there is no sentence end in reach. Trimming
/// untrusted text for a screen is what a quarantine preview already does; nothing here decides
/// anything from the bytes.
fn one_line(reply: &str) -> String {
    let flattened = reply.split_whitespace().collect::<Vec<_>>().join(" ");
    let said = past_the_preamble(&flattened);

    match first_sentence_end(said) {
        Some(at) if said[..at].chars().count() >= MIN_SUMMARY_CHARS => said[..at].to_string(),
        _ if said.chars().count() <= MAX_SUMMARY_CHARS => said.to_string(),
        _ => {
            let cut: String = said.chars().take(MAX_SUMMARY_CHARS).collect();
            let head = cut.rsplit_once(' ').map(|(h, _)| h).unwrap_or(&cut);
            format!("{head}…")
        }
    }
}

/// Where a stop ends a sentence, in bytes.
///
/// A stop followed by anything other than a space is not one. Without that, the dot in
/// "README.md" ends the sentence and a row reads "added the line to README".
fn first_sentence_end(text: &str) -> Option<usize> {
    text.char_indices()
        .filter(|(_, c)| matches!(c, '.' | '!' | '?'))
        .find(|(at, c)| {
            text[at + c.len_utf8()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        })
        .map(|(at, _)| at)
}

/// Where the substance of a reply starts.
///
/// A reply opens with a flourish and then says what it did: "Done! I've created haha.txt". Neither
/// half of that is worth a row. Dropped so the row starts at the verb, which is the word a person
/// scanning a list is looking for, and so every row does not begin the same way.
///
/// Only the opening is touched, and only where it is short enough to be a flourish rather than an
/// answer. Nothing is rewritten: what is left is the model's own words, and it is still marked as
/// the model's wherever it is drawn.
fn past_the_preamble(text: &str) -> &str {
    let mut rest = text;

    // "Done!", "Sure.", "Got it." A sentence this short before the real one says nothing.
    while let Some(at) = first_sentence_end(rest) {
        if rest[..at].chars().count() >= MIN_SUMMARY_CHARS {
            break;
        }
        let after = rest[at + 1..].trim_start();
        if after.is_empty() {
            break;
        }
        rest = after;
    }

    // "I've created X" says what "created X" says in half the width.
    for opener in [
        "I've ", "I have ", "I'll ", "I will ", "I ", "we've ", "We've ",
    ] {
        if let Some(said) = rest.strip_prefix(opener) {
            return said;
        }
    }
    rest
}

/// Where one session's checkpoints live.
///
/// Session-scoped rather than directory-scoped: two sessions working in one checkout keep separate
/// histories, so rewinding either never moves the other.
fn session_directory(project: &Path, session: &str) -> Option<PathBuf> {
    Some(
        crate::store::directory()?
            .join(CHECKPOINTS)
            .join(crate::sessions::key_for(project))
            .join(session),
    )
}

/// Seconds since the epoch, or zero where the clock is unreadable.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Create a file nobody else can read, whatever the umask would have given it.
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Accumulates what a turn changed, and writes it out when the turn ends.
///
/// Held by the interface rather than by the turn: the worker announces each write as it happens
/// and this decides what becomes of it, which is what keeps the location out of the crate that
/// carries the bytes.
///
/// Everything here degrades to doing nothing. No home directory, a read-only disk, a corrupt file:
/// none of that is worth failing a turn over, because the work succeeded and only the ability to
/// undo it was lost.
#[derive(Debug)]
pub struct Store {
    directory: Option<PathBuf>,
    /// What the turn in progress has changed so far.
    pending: Vec<Captured>,
    /// The model's account of the turn in progress, once it has finished saying it.
    did: Option<String>,
}

impl Store {
    /// A store with nowhere to write.
    ///
    /// What a session has before it knows its own name, and what every test that is not about
    /// storage uses. Every call still answers, so nothing has to check first.
    pub fn detached() -> Self {
        Self {
            directory: None,
            pending: Vec::new(),
            did: None,
        }
    }

    /// Open the store for one session, which may turn out to have nowhere to write.
    pub fn open(project: &Path, session: &str) -> Self {
        Self {
            directory: session_directory(project, session),
            pending: Vec::new(),
            did: None,
        }
    }

    /// Keep a file's prior contents, as announced by the turn that wrote over it.
    ///
    /// A path captured twice in one turn keeps the first, which is what the turn started from.
    /// The second is what this turn's own earlier write left, and returning to that would land
    /// halfway through the turn being discarded.
    pub fn capture(&mut self, written: bravebot_agent::report::Written) {
        if let Some(already) = self.pending.iter_mut().find(|c| c.path == written.path) {
            // The contents stay as the turn found them, but the change is the whole turn's: a
            // second write to one path added and removed lines too, and a row reporting only the
            // first would understate what returning here undoes.
            already.added += written.added;
            already.removed += written.removed;
            return;
        }
        let too_large = written
            .prior
            .as_ref()
            .is_some_and(|p| p.len() > MAX_FILE_BYTES);
        self.pending.push(Captured {
            path: written.path,
            prior: if too_large { None } else { written.prior },
            too_large,
            verb: written.verb.word().to_string(),
            added: written.added,
            removed: written.removed,
        });
    }

    /// Keep the model's account of the turn in progress, for the row to show.
    ///
    /// Untrusted, and stored as such. Taken from the reply the person was already shown, so the
    /// row says what they read rather than something said only to the list.
    pub fn summarised_by(&mut self, reply: &str) {
        let cut = one_line(reply);
        self.did = (!cut.is_empty()).then_some(cut);
    }

    /// Whether the turn in progress has changed anything worth keeping.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Write out what this turn changed, if anything, and start the next one empty.
    ///
    /// A turn that wrote no files leaves no checkpoint: a list of points that return to where you
    /// already are is a list nobody can use.
    pub fn finish_turn(&mut self, turn: usize) {
        let files = std::mem::take(&mut self.pending);
        let did = self.did.take();
        if files.is_empty() {
            return;
        }
        let Some(directory) = self.directory.clone() else {
            return;
        };
        if std::fs::create_dir_all(&directory).is_err() {
            return;
        }
        // One past the highest already kept, so a number is never reused inside a session even
        // after the oldest have been dropped.
        let number = self.list().first().map(|kept| kept.number).unwrap_or(0) + 1;
        let checkpoint = Checkpoint {
            number,
            turn,
            made: now(),
            did,
            files,
        };
        let Ok(encoded) = serde_json::to_string(&checkpoint) else {
            return;
        };
        let path = directory.join(format!("{number}.json"));
        if let Ok(mut file) = create_private(&path) {
            use std::io::Write;
            let _ = file.write_all(encoded.as_bytes());
        }
        self.prune();
    }

    /// Drop the oldest checkpoints once there are more than the bound allows.
    fn prune(&self) {
        let mut kept = self.list();
        if kept.len() <= MAX_CHECKPOINTS {
            return;
        }
        // Oldest first, so the ones to drop are at the front of what `list` returns reversed.
        kept.sort_by_key(|c| c.number);
        let Some(directory) = self.directory.as_ref() else {
            return;
        };
        for doomed in &kept[..kept.len() - MAX_CHECKPOINTS] {
            let _ = std::fs::remove_file(directory.join(format!("{}.json", doomed.number)));
        }
    }

    /// Every checkpoint this session can return to, most recent first.
    pub fn list(&self) -> Vec<Checkpoint> {
        let Some(directory) = self.directory.as_ref() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut found: Vec<Checkpoint> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .filter_map(|text| serde_json::from_str(&text).ok())
            .collect();
        found.sort_by_key(|found| std::cmp::Reverse(found.number));
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One file written over, as the turn would announce it.
    fn wrote(path: &str, prior: Option<&str>) -> bravebot_agent::report::Written {
        use bravebot_agent::report::{Written, WroteHow};
        Written {
            path: path.to_string(),
            prior: prior.map(str::to_string),
            verb: if prior.is_some() {
                WroteHow::Edited
            } else {
                WroteHow::Created
            },
            added: 1,
            removed: 1,
        }
    }

    /// A scratch directory of this test's own, so two tests cannot tread on each other.
    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bravebot-checkpoints-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create scratch");
        root
    }

    /// A store writing into a scratch directory.
    fn store_in(root: &Path) -> Store {
        Store {
            directory: Some(root.to_path_buf()),
            pending: Vec::new(),
            did: None,
        }
    }

    /// A store with nowhere to write, for the cases that never reach a disk.
    fn detached() -> Store {
        Store {
            directory: None,
            pending: Vec::new(),
            did: None,
        }
    }

    /// Losing the ability to undo a turn is not worth failing the turn over, so a store with
    /// nowhere to write still answers every call.
    #[test]
    fn a_store_with_no_directory_does_nothing_rather_than_failing() {
        let mut store = detached();
        store.capture(wrote("src/a.rs", Some("before")));
        assert!(store.has_pending());
        store.finish_turn(1);
        assert!(store.list().is_empty());
    }

    /// The distinction a restore turns on: a path that held no file is not a path that held an
    /// empty one, and putting an empty file back where there was none leaves an artefact behind.
    #[test]
    fn a_path_that_held_no_file_is_captured_as_holding_none() {
        let mut store = detached();
        store.capture(wrote("made.rs", None));
        store.capture(wrote("emptied.rs", Some("")));
        assert_eq!(store.pending[0].prior, None);
        assert_eq!(store.pending[1].prior, Some(String::new()));
    }

    /// A rewind returns to what the turn started from, so the first capture of a path is the one
    /// that counts: the second is what this turn's own earlier write left behind.
    #[test]
    fn capturing_one_path_twice_in_a_turn_keeps_what_the_turn_started_from() {
        let mut store = detached();
        store.capture(wrote("a.rs", Some("original")));
        store.capture(wrote("a.rs", Some("halfway")));
        assert_eq!(store.pending.len(), 1);
        assert_eq!(store.pending[0].prior, Some("original".to_string()));
    }

    /// A file too large to copy is still recorded as changed, so the list does not quietly omit
    /// it and a restore can refuse rather than put back a truncation.
    #[test]
    fn a_file_too_large_to_copy_is_recorded_without_its_contents() {
        let mut store = detached();
        store.capture(wrote("big.rs", Some(&"x".repeat(MAX_FILE_BYTES + 1))));
        assert!(store.pending[0].too_large);
        assert_eq!(store.pending[0].prior, None);
    }

    /// A list of points that return to where you already are is a list nobody can use.
    #[test]
    fn a_turn_that_wrote_nothing_leaves_no_checkpoint() {
        let root = scratch("empty-turn");
        let mut store = store_in(&root);
        store.finish_turn(1);
        assert!(store.list().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_a_turn_changed_is_written_and_read_back() {
        let root = scratch("round-trip");
        let mut store = store_in(&root);
        store.capture(wrote("src/a.rs", Some("before")));
        store.capture(wrote("src/new.rs", None));
        store.finish_turn(4);

        let listed = store.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].turn, 4);
        assert_eq!(listed[0].files.len(), 2);
        assert_eq!(listed[0].files[0].prior, Some("before".to_string()));
        assert_eq!(listed[0].files[1].prior, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A rewind reaches for something recent, so the newest is what a person sees first.
    #[test]
    fn a_list_puts_the_most_recent_checkpoint_first() {
        let root = scratch("ordering");
        let mut store = store_in(&root);
        for turn in 1..=3 {
            store.capture(wrote(&format!("{turn}.rs"), Some("x")));
            store.finish_turn(turn);
        }
        let turns: Vec<usize> = store.list().iter().map(|c| c.turn).collect();
        assert_eq!(turns, vec![3, 2, 1]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The bound exists so a long session cannot grow without limit. It drops the oldest rather
    /// than refusing a new one, which would withdraw the protection exactly when it is needed.
    #[test]
    fn passing_the_bound_drops_the_oldest_rather_than_refusing_a_new_one() {
        let root = scratch("bounded");
        let mut store = store_in(&root);
        let last = MAX_CHECKPOINTS + 5;
        for turn in 1..=last {
            store.capture(wrote(&format!("{turn}.rs"), Some("x")));
            store.finish_turn(turn);
        }
        let listed = store.list();
        assert_eq!(listed.len(), MAX_CHECKPOINTS);
        assert_eq!(listed[0].turn, last);
        assert!(listed.iter().all(|c| c.turn > 5));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A row says what the turn did to the file, which is what returning here would undo.
    /// "Turn 8" says when and nothing about what.
    #[test]
    fn a_row_says_what_was_done_and_how_much() {
        let root = scratch("what-was-done");
        let mut store = store_in(&root);
        store.capture(wrote("src/auth.rs", Some("before")));
        store.finish_turn(8);

        let (summary, untrusted) = store.list()[0].summary();
        assert!(!untrusted, "the driver's own record was marked untrusted");
        assert!(summary.contains("src/auth.rs"), "{summary}");
        assert!(summary.contains("+1"), "{summary}");
        assert!(summary.contains("-1"), "{summary}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A creation removes nothing and adds every line, so counting them says nothing a person
    /// wants: the word and the name are the whole story.
    #[test]
    fn a_created_file_says_so_rather_than_counting_its_lines() {
        let created = Checkpoint {
            number: 7,
            turn: 7,
            made: 0,
            did: None,
            files: vec![Captured {
                path: "src/auth.rs".to_string(),
                prior: None,
                too_large: false,
                verb: "created".to_string(),
                added: 3,
                removed: 0,
            }],
        };
        assert_eq!(
            created.summary(),
            ("created src/auth.rs".to_string(), false)
        );
    }

    /// A turn that touched several files names one and counts the rest, because a row has to fit
    /// on a line and the first file is the one the turn was mostly about.
    #[test]
    fn a_turn_that_changed_several_files_names_one_and_counts_the_rest() {
        let root = scratch("several");
        let mut store = store_in(&root);
        store.capture(wrote("a.rs", Some("x")));
        store.capture(wrote("b.rs", Some("y")));
        store.capture(wrote("c.rs", Some("z")));
        store.finish_turn(2);

        let (summary, _) = store.list()[0].summary();
        assert!(summary.contains("a.rs"), "{summary}");
        assert!(
            summary.contains('2'),
            "{summary} does not count the other two"
        );
        assert!(!summary.contains("b.rs"), "{summary} names more than one");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Writing the same path twice in a turn keeps the contents the turn started from, and the
    /// change is the whole turn's: reporting only the first write would understate the undo.
    #[test]
    fn two_writes_to_one_path_add_up_to_one_row() {
        let root = scratch("added-up");
        let mut store = store_in(&root);
        store.capture(wrote("a.rs", Some("original")));
        store.capture(wrote("a.rs", Some("halfway")));
        store.finish_turn(1);

        let listed = store.list();
        assert_eq!(listed[0].files.len(), 1);
        assert_eq!(listed[0].files[0].prior, Some("original".to_string()));
        assert_eq!(
            listed[0].files[0].added, 2,
            "the second write was not counted"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Asking twice is how a person checks whether a turn left a point behind, so the answer
    /// replaces the one before it rather than stacking below it.
    #[test]
    fn asking_twice_leaves_one_list_in_the_transcript() {
        let mut session = crate::state::Session::new("none");
        let listed = vec![Checkpoint {
            number: 1,
            turn: 1,
            made: now(),
            did: Some("created a.rs".to_string()),
            files: Vec::new(),
        }];

        session.show_checkpoints(rows(&listed));
        session.show_checkpoints(rows(&listed));
        session.show_checkpoints(rows(&listed));

        let lists = session
            .transcript
            .iter()
            .filter(|entry| !entry.checkpoints.is_empty())
            .count();
        assert_eq!(lists, 1, "the list stacked up instead of replacing");
    }

    /// Replacing the list must not take the transcript with it.
    #[test]
    fn replacing_the_list_leaves_the_rest_of_the_transcript_alone() {
        let mut session = crate::state::Session::new("none");
        session.note("something said earlier");
        let listed = vec![Checkpoint {
            number: 1,
            turn: 1,
            made: now(),
            did: Some("created a.rs".to_string()),
            files: Vec::new(),
        }];

        session.show_checkpoints(rows(&listed));
        session.show_checkpoints(rows(&listed));

        let kept = session
            .transcript
            .iter()
            .any(|entry| entry.text.contains("something said earlier"));
        assert!(kept, "replacing the list dropped the transcript");
    }

    /// One report line per checkpoint, so a list of one cannot draw two rows.
    #[test]
    fn each_checkpoint_is_one_row_and_no_more() {
        let root = scratch("one-row");
        let mut store = store_in(&root);
        store.capture(wrote("a.rs", Some("x")));
        store.finish_turn(1);

        let listed = store.list();
        assert_eq!(listed.len(), 1, "one turn wrote more than one checkpoint");
        assert_eq!(rows(&listed).len(), 1, "one checkpoint drew two rows");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The numbering counts from the newest rather than from the turn, so the row a person
    /// reaches for is 1 whatever turn the session is on.
    #[test]
    fn a_row_keeps_its_own_number_however_many_arrive() {
        let root = scratch("numbering");
        let mut store = store_in(&root);
        for turn in [3, 9] {
            store.capture(wrote(&format!("{turn}.rs"), Some("x")));
            store.finish_turn(turn);
        }

        // Newest first, and each row carries its own number rather than its place in the list.
        let listed = rows(&store.list());
        assert_eq!(listed[0].number, "2.");
        assert_eq!(listed[1].number, "1.");

        // A third arriving does not renumber the two already there, which is what makes a number
        // worth quoting back.
        store.capture(wrote("later.rs", Some("x")));
        store.finish_turn(11);
        let listed = rows(&store.list());
        assert_eq!(listed[0].number, "3.");
        assert_eq!(listed[1].number, "2.");
        assert_eq!(listed[2].number, "1.");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The turn is not what a row says. Turns skip whenever one changes no file, and a list with
    /// 1, 2, 4 in it invites a person to wonder what happened to 3.
    #[test]
    fn a_row_says_which_checkpoint_and_never_which_turn() {
        let root = scratch("no-turns");
        let mut store = store_in(&root);
        // Turn 3 changed a file, turns 4 and 5 changed nothing, turn 6 changed one.
        store.capture(wrote("a.rs", Some("x")));
        store.finish_turn(3);
        store.finish_turn(4);
        store.finish_turn(5);
        store.capture(wrote("b.rs", Some("y")));
        store.finish_turn(6);

        let listed = rows(&store.list());
        assert_eq!(
            listed.len(),
            2,
            "a turn that changed nothing left a checkpoint"
        );
        assert_eq!(listed[0].number, "2.");
        assert_eq!(listed[1].number, "1.");
        for row in &listed {
            assert!(
                !row.when.to_lowercase().contains("turn"),
                "a row still mentions a turn: {}",
                row.when
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The model's account of a turn reads best and is the model describing its own work, so the
    /// row says it is untrusted and the renderer marks it.
    #[test]
    fn a_summary_the_model_wrote_is_reported_as_untrusted() {
        let root = scratch("model-summary");
        let mut store = store_in(&root);
        store.capture(wrote("auth.rs", Some("before")));
        store.summarised_by("Implemented login validation. Also tidied the imports.");
        store.finish_turn(8);

        let (summary, untrusted) = store.list()[0].summary();
        assert_eq!(summary, "Implemented login validation");
        assert!(untrusted, "the model's own words were not marked untrusted");

        let row = &rows(&store.list())[0];
        assert!(row.untrusted);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A reply runs for paragraphs and a row is one line, so it is cut at the first sentence.
    #[test]
    fn a_reply_is_cut_at_its_first_sentence() {
        assert_eq!(
            one_line("Added the line to README.md. Then I verified it by re-reading the file."),
            "Added the line to README.md"
        );
    }

    /// A reply that opens with a filler sentence says nothing, so the stop is passed over. This
    /// is what a real reply looked like, and the row read "Done".
    #[test]
    fn a_reply_opening_with_done_does_not_become_a_row_saying_done() {
        let cut = one_line(
            "Done! I've added the line \"this is a test\" at the end of the README.md file.",
        );
        assert_ne!(cut, "Done");
        assert!(cut.contains("this is a test"), "{cut}");
    }

    /// A row starts at what was done, because that is the word a person scanning a list is
    /// looking for. This is what a real reply looked like, and the row read "Done! I've created".
    #[test]
    fn a_row_starts_at_what_was_done() {
        assert_eq!(
            one_line("Done! I've created `haha.txt` with the content \"haha I am here\""),
            "created `haha.txt` with the content \"haha I am here\""
        );
    }

    /// Every row would otherwise open the same way, which is width spent saying nothing.
    #[test]
    fn a_first_person_opening_is_dropped() {
        assert_eq!(
            one_line("I have added a validate_login function to auth.rs"),
            "added a validate_login function to auth.rs"
        );
        assert_eq!(
            one_line("I added the line to the end of README.md"),
            "added the line to the end of README.md"
        );
    }

    /// A reply that already begins with what it did is left as it is.
    #[test]
    fn a_reply_already_starting_at_the_verb_is_left_alone() {
        assert_eq!(
            one_line("Added the line to README.md. Then verified it."),
            "Added the line to README.md"
        );
    }

    /// Nothing but a flourish leaves the flourish, since a row saying nothing at all is worse.
    #[test]
    fn a_reply_that_is_only_a_flourish_keeps_it() {
        assert_eq!(one_line("Done!"), "Done!");
    }

    /// A reply with no sentence end in reach is cut at a word rather than mid-word, and says it
    /// was cut.
    #[test]
    fn a_reply_with_no_sentence_end_is_cut_at_a_word() {
        let cut = one_line(&"verylongword ".repeat(20));
        assert!(cut.ends_with('…'), "{cut}");
        assert!(cut.chars().count() <= MAX_SUMMARY_CHARS + 1, "{cut}");
        assert!(!cut.contains("verylongwordverylongword"), "{cut}");
    }

    /// Newlines in a reply would otherwise break the row it is drawn on.
    #[test]
    fn a_reply_spanning_lines_becomes_one_line() {
        assert_eq!(
            one_line("Added a line\n\nto README md"),
            "Added a line to README md"
        );
    }

    /// A checkpoint holds the contents of source files, so the mode is chosen here rather than
    /// left to whatever the umask happens to be.
    #[cfg(unix)]
    #[test]
    fn a_checkpoint_is_written_readable_by_nobody_else() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("private");
        let mut store = store_in(&root);
        store.capture(wrote("a.rs", Some("secret")));
        store.finish_turn(1);

        let mode = std::fs::metadata(root.join("1.json"))
            .expect("the checkpoint was written")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Two sessions in one checkout keep separate histories, so rewinding either never moves the
    /// other, and neither offers the other's checkpoints to return to.
    #[test]
    fn two_sessions_in_one_directory_do_not_share_checkpoints() {
        let project = Path::new("/tmp/some-project");
        let (Some(one), Some(two)) = (
            session_directory(project, "session-one"),
            session_directory(project, "session-two"),
        ) else {
            // No home directory on this machine, which is a different property.
            return;
        };
        assert_ne!(one, two);
        assert_eq!(one.parent(), two.parent());
    }

    /// Nothing to return to draws no rows, so the interface says why instead of framing an
    /// empty list.
    #[test]
    fn a_session_with_nothing_to_return_to_draws_no_rows() {
        assert!(rows(&[]).is_empty());
    }

    /// A list is scrolled past, so the contents of the files stay out of it.
    #[test]
    fn a_list_shows_no_file_contents() {
        let secret = "a-secret-nobody-should-see-in-a-list";
        let listed = vec![Checkpoint {
            number: 1,
            turn: 1,
            made: now(),
            did: None,
            files: vec![Captured {
                path: "a.rs".to_string(),
                prior: Some(secret.to_string()),
                too_large: false,
                verb: "edited".to_string(),
                added: 1,
                removed: 1,
            }],
        }];
        for row in rows(&listed) {
            assert!(!row.number.contains(secret));
            assert!(!row.summary.contains(secret), "{}", row.summary);
            assert!(!row.when.contains(secret));
        }
    }

    /// The record is read back into a later turn's context and this is not, so the two must not
    /// share a tree.
    #[test]
    fn checkpoints_do_not_live_under_the_session_records() {
        let project = Path::new("/tmp/some-project");
        let (Some(checkpoints), Some(records)) = (
            session_directory(project, "a-session"),
            crate::sessions::project_directory(project),
        ) else {
            return;
        };
        assert!(
            !checkpoints.starts_with(&records),
            "{checkpoints:?} is inside {records:?}"
        );
    }
}
