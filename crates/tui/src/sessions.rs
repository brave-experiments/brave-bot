//! Sessions kept on disk, so one can be picked up again tomorrow.
//!
//! Under `~/.bua/sessions`, one directory per working directory, because a session belongs to
//! the checkout it happened in: the list worth seeing when resuming in one project is not the
//! list from another. The directory is named after the path it stands for, mangled into one
//! segment, and the real path is written inside the record as well, since the mangling is not
//! reversible.
//!
//! Two files per session. The **record** holds what the picker shows and what a resume needs:
//! the conversation, and nothing else about it. The **audit** holds every gate decision the
//! session made, one JSON object per line, which is the file to read when the question is what
//! the agent was allowed to do and why.
//!
//! # What is written, and what is not
//!
//! Every message in the record has already been past the present gate, so what lands on disk is
//! what the planner was allowed to hold: no untrusted bytes, by construction rather than by
//! filtering. The quarantine is not written at all, and the audit is labels and gate names with
//! no content in it. See [`bua_agent::conversation::Snapshot`].
//!
//! Everything degrades to doing nothing. A missing home, a full disk, a corrupt record: a
//! session that cannot be written down still runs, and one that cannot be read is left out of
//! the list rather than taken as a reason to fail.

use bua_agent::conversation::Snapshot;
use bua_core::event::Event;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where sessions live inside the state directory.
const SESSIONS: &str = "sessions";

/// A session as it is written down.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    /// Unique within its project directory, and sortable, since it starts with the time.
    pub id: String,
    /// The working directory the session ran in, as it was.
    pub directory: String,
    /// The branch checked out at the time, where there was one.
    #[serde(default)]
    pub branch: Option<String>,
    /// What to call it in a list: the first thing the user asked.
    pub title: String,
    /// When it began and when it was last written, in seconds since the epoch.
    pub started: u64,
    pub updated: u64,
    /// How many turns it has had, for a reader of the file.
    #[serde(default)]
    pub turns: usize,
    /// The conversation, which is what resuming restores.
    pub conversation: Snapshot,
}

/// One line of the list, without the conversation behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub branch: Option<String>,
    pub updated: u64,
    /// What the session takes up, record and audit together.
    pub bytes: u64,
}

/// A live session, holding where to write and what has been written.
#[derive(Debug, Clone)]
pub struct Handle {
    id: String,
    project: PathBuf,
    started: u64,
    branch: Option<String>,
    title: String,
}

impl Handle {
    /// Begin a session for work in `project`.
    ///
    /// Nothing is written yet: a session that is opened and abandoned should not leave a record,
    /// or the list fills with launches nobody meant.
    pub fn begin(project: &Path) -> Self {
        Self {
            id: new_id(),
            project: project.to_path_buf(),
            started: now(),
            branch: branch_of(project),
            title: String::new(),
        }
    }

    /// Continue the session a record came from, writing back to the same files.
    pub fn resuming(project: &Path, record: &Record) -> Self {
        Self {
            id: record.id.clone(),
            project: project.to_path_buf(),
            started: record.started,
            branch: branch_of(project),
            title: record.title.clone(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Write the session down as it now stands.
    ///
    /// Called after each turn rather than at the end, because the end may never come: a session
    /// that was killed, or whose machine slept and never woke, is exactly the one worth
    /// resuming.
    pub fn save(&mut self, conversation: &Snapshot, turns: usize, first_prompt: &str) {
        if self.title.is_empty() {
            self.title = title_from(first_prompt);
        }

        let Some(directory) = self.directory() else {
            return;
        };

        let record = Record {
            id: self.id.clone(),
            directory: self.project.display().to_string(),
            branch: self.branch.clone(),
            title: self.title.clone(),
            started: self.started,
            updated: now(),
            turns,
            conversation: conversation.clone(),
        };

        let Ok(body) = serde_json::to_vec_pretty(&record) else {
            return;
        };

        // Written beside and renamed, so a session killed mid-write leaves the last good record
        // rather than half of a new one.
        let temporary = directory.join(format!("{}.tmp", self.id));
        if std::fs::write(&temporary, body).is_ok() {
            let _ = std::fs::rename(&temporary, directory.join(format!("{}.json", self.id)));
        }
    }

    /// Append what one turn's gates decided.
    ///
    /// One JSON object per line, so the file can be grown a turn at a time and read with
    /// ordinary tools. What goes in it is gate names, labels and paths: the audit says what was
    /// allowed and why, never what the content was.
    pub fn append_audit(&self, turn: usize, events: &[Event]) {
        let Some(directory) = self.directory() else {
            return;
        };
        let at = now();

        let mut body = String::new();
        for event in events {
            let line = serde_json::json!({
                "at": at,
                "turn": turn,
                "event": crate::audit::as_json(event),
            });
            body.push_str(&line.to_string());
            body.push('\n');
        }

        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join(format!("{}.audit.jsonl", self.id)))
            .and_then(|mut file| file.write_all(body.as_bytes()));
    }

    /// The directory to write into, made on first use.
    fn directory(&self) -> Option<PathBuf> {
        let directory = project_directory(&self.project)?;
        std::fs::create_dir_all(&directory).ok()?;
        Some(directory)
    }
}

/// Sessions for this project, newest first.
///
/// A record that will not parse is left out rather than reported: an unreadable file should cost
/// its own line in the list and nothing more.
pub fn list(project: &Path) -> Vec<Summary> {
    let Some(directory) = project_directory(project) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Vec::new();
    };

    let mut summaries: Vec<Summary> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|e| e == "json"))
        .filter_map(|entry| {
            let record = read(&entry.path())?;
            let audit = directory.join(format!("{}.audit.jsonl", record.id));
            let bytes = size_of(&entry.path()) + size_of(&audit);
            Some(Summary {
                id: record.id,
                title: record.title,
                branch: record.branch,
                updated: record.updated,
                bytes,
            })
        })
        .collect();

    summaries.sort_by(|a, b| b.updated.cmp(&a.updated));
    summaries
}

/// Read one session back, by the id the list gave.
pub fn load(project: &Path, id: &str) -> Option<Record> {
    let directory = project_directory(project)?;
    read(&directory.join(format!("{id}.json")))
}

/// What each turn of a stored session left in the audit, by turn number.
///
/// The record holds the conversation and the audit holds what the gates decided, so a resumed
/// session that reads only the record shows a transcript with the trail missing under every turn
/// that happened before the resume. The data was never lost; it was simply never read back.
///
/// Empty for a session with no audit file, which is the ordinary case for one that never ran a
/// turn. A line that will not parse is skipped rather than reported: a trail is read to answer a
/// question about what happened, and one unreadable line should cost its own line and no more.
pub fn audit_of(project: &Path, id: &str) -> BTreeMap<usize, Vec<crate::audit::TrailLine>> {
    let mut trails: BTreeMap<usize, Vec<crate::audit::TrailLine>> = BTreeMap::new();
    let Some(directory) = project_directory(project) else {
        return trails;
    };
    let Ok(contents) = std::fs::read_to_string(directory.join(format!("{id}.audit.jsonl"))) else {
        return trails;
    };

    for line in contents.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(turn) = entry["turn"].as_u64() else {
            continue;
        };
        if let Some(recorded) = crate::audit::recalled(&entry["event"]) {
            trails.entry(turn as usize).or_default().push(recorded);
        }
    }
    trails
}

/// Where a project's sessions live.
pub fn project_directory(project: &Path) -> Option<PathBuf> {
    Some(
        crate::store::directory()?
            .join(SESSIONS)
            .join(key_for(project)),
    )
}

/// The single path segment standing for a working directory.
///
/// Separators become dashes and anything that is not a plain path character goes the same way,
/// so the name is one segment on every platform and readable in a directory listing. It is not
/// reversible, which is why the record holds the real path as well.
pub fn key_for(project: &Path) -> String {
    let mangled: String = project
        .display()
        .to_string()
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' => c,
            _ => '-',
        })
        .collect();

    // A path of only separators would otherwise name the sessions directory itself.
    if mangled.trim_matches('-').is_empty() {
        return "root".to_string();
    }
    mangled
}

/// The branch checked out in `directory`, where it is a git checkout at all.
///
/// Read out of the files rather than by running git: it is one line, and this is a label on a
/// list entry. A detached head has no branch name, which is reported as none rather than as the
/// commit it happens to be on.
pub fn branch_of(directory: &Path) -> Option<String> {
    let git = find_git(directory)?;
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let reference = head.trim().strip_prefix("ref: ")?;
    let branch = reference.strip_prefix("refs/heads/")?;
    Some(branch.to_string())
}

/// The git directory for a checkout, walking up from `directory`.
fn find_git(directory: &Path) -> Option<PathBuf> {
    for candidate in directory.ancestors() {
        let git = candidate.join(".git");
        if git.is_dir() {
            return Some(git);
        }
        // A worktree or a submodule has a file pointing at the real directory.
        if git.is_file()
            && let Ok(contents) = std::fs::read_to_string(&git)
            && let Some(path) = contents.trim().strip_prefix("gitdir: ")
        {
            return Some(candidate.join(path));
        }
    }
    None
}

/// A title from the first thing the user asked.
///
/// Its first line, shortened. A prompt is what the session was about, and a session named after
/// one is findable in a way that a timestamp is not.
pub fn title_from(prompt: &str) -> String {
    const LONGEST: usize = 60;

    let line = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.trim();
    if line.is_empty() {
        return "untitled".to_string();
    }

    let mut title: String = line.chars().take(LONGEST).collect();
    if line.chars().count() > LONGEST {
        title.push('…');
    }
    title
}

/// How long ago, in the words a list uses.
///
/// The phrasing is the agent crate's, so a session last touched thirteen minutes ago and a file
/// replaced thirteen minutes ago are described the same way.
pub fn how_long_ago(then: u64) -> String {
    let seconds = now().saturating_sub(then);
    bua_agent::report::how_long_ago(std::time::Duration::from_secs(seconds))
}

/// A size in the units a person reads.
pub fn size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;

    let bytes = bytes as f64;
    if bytes >= MB {
        format!("{:.1}MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes / KB)
    } else {
        format!("{bytes:.0}B")
    }
}

fn read(path: &Path) -> Option<Record> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn size_of(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// An id that sorts by when it was made and cannot collide with another process's.
fn new_id() -> String {
    format!("{}-{}", now(), std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_working_directory_becomes_one_readable_segment() {
        let key = key_for(Path::new("/Users/someone/projects/bua"));
        assert!(!key.contains('/'));
        assert!(key.contains("projects"), "{key} is not recognisable");
    }

    /// Two checkouts must not share a list, which is the whole point of keying by directory.
    #[test]
    fn two_directories_do_not_share_a_key() {
        assert_ne!(key_for(Path::new("/a/one")), key_for(Path::new("/a/two")));
    }

    /// A path of nothing but separators would otherwise name the sessions directory itself and
    /// scatter records among the project directories.
    #[test]
    fn a_path_with_nothing_in_it_still_names_a_directory() {
        assert_eq!(key_for(Path::new("/")), "root");
    }

    #[test]
    fn a_title_is_the_first_line_of_the_prompt() {
        assert_eq!(
            title_from("make a space invaders game\nwith canvas"),
            "make a space invaders game"
        );
    }

    /// A pasted essay is not a title, and cutting it says so rather than pretending.
    #[test]
    fn a_long_title_is_cut_and_says_it_was() {
        let title = title_from(&"x".repeat(200));
        assert!(title.ends_with('…'));
        assert!(title.chars().count() <= 61);
    }

    #[test]
    fn a_prompt_with_nothing_in_it_still_has_a_title() {
        assert_eq!(title_from("   \n\n"), "untitled");
    }

    /// The phrasing itself is tested where it lives; what matters here is that a stored time
    /// becomes an age rather than being read as one.
    #[test]
    fn a_stored_time_becomes_an_age() {
        let now = now();
        assert_eq!(how_long_ago(now), "just now");
        assert_eq!(how_long_ago(now - 13 * 60), "13 minutes ago");
    }

    /// A clock that has gone backwards since the session was written must not produce an age in
    /// the future or a panic.
    #[test]
    fn a_session_from_the_future_is_not_a_crash() {
        assert_eq!(how_long_ago(now() + 10_000), "just now");
    }

    #[test]
    fn sizes_read_the_way_a_person_says_them() {
        assert_eq!(size(512), "512B");
        assert_eq!(size(182_374), "178.1KB");
        assert_eq!(size(3_774_874), "3.6MB");
    }

    /// Two sessions started in the same second must not write to the same pair of files. They
    /// are different processes, which is what the second half of the id says.
    #[test]
    fn an_id_names_the_process_as_well_as_the_time() {
        let id = new_id();
        let (time, process) = id.split_once('-').expect("an id has both halves");
        assert!(
            time.parse::<u64>().is_ok(),
            "{id} does not start with a time"
        );
        assert_eq!(process, std::process::id().to_string());
    }

    /// A scratch checkout, so the test says what the code reads rather than what this machine
    /// happens to have checked out.
    fn fake_checkout(name: &str, head: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bua-sessions-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).expect("create scratch");
        std::fs::write(root.join(".git").join("HEAD"), head).expect("write HEAD");
        root
    }

    #[test]
    fn the_branch_is_read_out_of_the_checkout() {
        let root = fake_checkout("branch", "ref: refs/heads/main\n");
        assert_eq!(branch_of(&root), Some("main".to_string()));

        // And from a directory inside it, since that is where a session usually runs.
        let inside = root.join("crates").join("tui");
        std::fs::create_dir_all(&inside).expect("create");
        assert_eq!(branch_of(&inside), Some("main".to_string()));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A detached head is on no branch, and reporting the commit it happens to be on would put a
    /// forty-character hex string where a name goes.
    #[test]
    fn a_detached_head_has_no_branch_name() {
        let root = fake_checkout("detached", "9fceb02d0ae598e95dc970b74767f19372d61af8\n");
        assert_eq!(branch_of(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Not every directory is a checkout, and that is not a failure to report.
    #[test]
    fn a_directory_that_is_not_a_checkout_has_no_branch() {
        let root = std::env::temp_dir().join("bua-sessions-not-a-checkout");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create scratch");
        assert_eq!(branch_of(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
