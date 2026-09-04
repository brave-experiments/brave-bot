//! Sessions kept on disk, so one can be picked up again tomorrow.
//!
//! Under `~/.bravebot/sessions`, one directory per working directory, because a session belongs to
//! the checkout it happened in: the list worth seeing when resuming in one project is not the
//! list from another. The directory is named after the path it stands for, mangled into one
//! segment, and the real path is written inside the record as well, since the mangling is not
//! reversible.
//!
//! Two files per session. The **record** holds what the picker shows and what a resume needs:
//! the conversation, and what the transcript showed beside it, which is the plan each turn worked
//! to and what the whole session has spent. The **audit** holds every gate decision the session
//! made, one JSON object per line, which is the file to read when the question is what the agent
//! was allowed to do and why. It is also read back on a resume, since a trail under the turns
//! from this process and nothing under the earlier ones is a worse account than either.
//!
//! The trust map is in the record too, and belongs there rather than to the directory: a map kept
//! per directory would answer the startup question for a user who was never asked. A fresh session
//! asks; a resumed one inherits what its own user answered.
//!
//! # What is written, and what is not
//!
//! Every message in the record has already been past the present gate, so what lands on disk is
//! what the planner was allowed to hold: no untrusted bytes, by construction rather than by
//! filtering. The same goes for the task lists, which came out of the render gate on their way to
//! the screen. The quarantine is not written at all, and the audit is labels and gate names with
//! no content in it. See [`bravebot_agent::conversation::Snapshot`].
//!
//! Everything degrades to doing nothing. A missing home, a full disk, a corrupt record: a
//! session that cannot be written down still runs, and one that cannot be read is left out of
//! the list rather than taken as a reason to fail.

use bravebot_agent::conversation::Snapshot;
use bravebot_agent::workspace::Workspace;
use bravebot_core::label::Integrity;
use bravebot_core::programs::TrustedPrograms;
use bravebot_core::todo::{self, Item, List, Row, Status};
use bravebot_core::trust::TrustStore;
use bravebot_i18n::t;
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
    /// What the session has spent, in tokens, across every turn it has had.
    ///
    /// The figure answers "what has this cost me", which is a question about the session rather
    /// than about the process that happened to be running it.
    #[serde(default)]
    pub tokens: u64,
    /// The model the server reported answering with, as of the last turn.
    ///
    /// What answered rather than what was asked for, since those differ: an endpoint may serve
    /// something other than the name it was given, and the one that did the work is the one a
    /// reader of this file needs. The name asked for is not kept, being a setting rather than a
    /// fact about the session.
    ///
    /// `None` for a record written before this was kept, or one whose turns never reached a
    /// server.
    #[serde(default)]
    pub model: Option<String>,
    /// What each turn cost, by turn number.
    ///
    /// The total says what the session cost; this says which turn cost it. Reading a session back
    /// to find out why it was expensive, a total cannot tell twenty even turns from one that ran
    /// away, and those are different problems.
    ///
    /// Empty for a record written before this was kept, which needs no question: the total is
    /// still there and only the breakdown is missing.
    #[serde(default)]
    pub spend: BTreeMap<usize, u64>,
    /// Where each turn's wall clock went, by turn number.
    ///
    /// The other half of the question [`Record::spend`] answers. Tokens say what a turn cost the
    /// endpoint; this says what it cost the person sitting in front of it, and the two need not
    /// agree at all: the cheapest turn in a session is often the one that stopped, put a command on
    /// the screen, and waited twenty minutes to be allowed to run it.
    ///
    /// Empty for a record written before this was kept, which is not the same as a session that
    /// took no time.
    #[serde(default)]
    pub timing: BTreeMap<usize, bravebot_agent::timing::Timing>,
    /// The task list each turn worked to, by turn number.
    ///
    /// Kept per turn rather than as one list, because that is how the transcript shows it: the
    /// plan a turn set out with sits beneath what that turn produced.
    #[serde(default)]
    pub todos: BTreeMap<usize, Vec<StoredTask>>,
    /// Which paths this session's user vouched for, and what its writes recorded since.
    ///
    /// Belongs to the session rather than to the directory, and that is the whole point. A map
    /// kept per directory would answer the startup question on behalf of a user who was never
    /// asked, which is trust assumed from silence. Resuming inherits it because the person
    /// resuming is the person who gave it; a session started fresh in the same directory is
    /// asked, and answers for itself.
    ///
    /// `None` for a record written before this was kept, which is asked about rather than read
    /// as an empty map: nothing recorded is not the same as nothing trusted.
    #[serde(default)]
    pub trust: Option<Vec<StoredRule>>,
    /// Which commands this session's user vouched for: resolved path and exact arguments.
    ///
    /// Belongs to the session for the same reason the trust map does: vouching for a command is a
    /// standing permission over both its side effects and the trust of its output, and a list kept
    /// per directory would grant that on behalf of a user who was never asked. Resuming inherits
    /// it because the person resuming is the person who gave it.
    ///
    /// Absent, unlike the map, needs no question: an empty list means every run asks and no
    /// output is trusted, which is what a session that recorded nothing should do.
    #[serde(default)]
    pub programs: Vec<StoredCommand>,
    /// Which directories this session opened with `/add-dir`, canonical.
    ///
    /// The trust map carries only half of what `/add-dir` did. The other half is reachability: an
    /// absolute path is refused whatever the map says unless the directory is open, so a record
    /// with the rule and not the directory resumes holding a rule about files nothing can open.
    /// Both halves are written, and both come back together.
    ///
    /// Empty for a record written before this was kept, which needs no question: such a session
    /// resumes with the rules it recorded and nothing open, exactly as it did before.
    #[serde(default)]
    pub directories: Vec<String>,
    /// Which build wrote this record: the version, the commit, and whether the tree was
    /// modified. See [`crate::BUILD`].
    ///
    /// A transcript is read after the fact, usually because something in it went wrong, and the
    /// first question is whether the code that produced it is the code in front of you. Without
    /// this that has to be inferred from the transcript's own symptoms.
    ///
    /// `None` for a record written before this was kept.
    #[serde(default)]
    pub build: Option<String>,
    /// The conversation, which is what resuming restores.
    pub conversation: Snapshot,
    /// What a manifest run produced, where this was one.
    ///
    /// Its presence is what makes a record a manifest run, and its absence a turn session. A
    /// manifest run has no conversation to restore, so [`Record::conversation`] is empty for one
    /// and resuming is refused rather than attempted. What it has instead is this: the goal in
    /// plain words, the manifest the planner proposed, the plan that was frozen, and what each
    /// step did.
    ///
    /// Written whether the run finished or failed, because the run somebody needs to read is the
    /// one that stopped.
    #[serde(default)]
    pub manifest: Option<StoredManifest>,
}

/// A manifest run as it is written down.
///
/// The same fields as `bravebot_agent::manifest::Attempt`, flattened for the file. Kept as its own
/// type rather than serialising the agent's, because a record on disk outlives the shape of a
/// struct in memory and a `#[serde(default)]` field here is cheaper than a migration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredManifest {
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub proposed: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    /// Why the run stopped, where it did.
    #[serde(default)]
    pub failure: Option<String>,
}

impl StoredManifest {
    /// Write down what a run produced, finished or not.
    pub fn of(attempt: &bravebot_agent::manifest::Attempt, failure: Option<String>) -> Self {
        Self {
            shape: attempt.shape.clone(),
            proposed: attempt.proposed.clone(),
            plan: attempt.plan.clone(),
            steps: attempt.steps.clone(),
            failure,
        }
    }

    /// How the record reads, for `--resume` of a run that cannot be continued.
    ///
    /// The picker only has a footer. Naming the session on the command line is the way to see
    /// what it produced, so this is the same report a failed live run printed, plus why it
    /// stopped where that was kept.
    pub fn describe(&self) -> String {
        let mut out = bravebot_agent::manifest::Attempt {
            shape: self.shape.clone(),
            proposed: self.proposed.clone(),
            plan: self.plan.clone(),
            steps: self.steps.clone(),
        }
        .describe();
        if let Some(failure) = &self.failure {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(failure);
            out.push('\n');
        }
        out
    }
}

/// One trust rule as it is written down.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRule {
    pub path: String,
    pub integrity: String,
}

/// The word for a trusted rule. Anything else reads as untrusted.
const TRUSTED: &str = "trusted";
const UNTRUSTED: &str = "untrusted";

/// One vouched-for command as it is written down.
///
/// The arguments are kept as a list rather than joined into a line, because they are matched
/// exactly and a rendering that had to be re-split would be a parser deciding what the user
/// vouched for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCommand {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// One task as it is written down.
///
/// The status is a word rather than the row's marker, because the marker is a glyph this build
/// chose and the status is what the model said. Rebuilding the row from the status means a
/// resumed list is drawn by the same code that draws a live one, so the two cannot diverge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTask {
    pub content: String,
    pub status: String,
}

impl StoredTask {
    fn of(row: &Row) -> Self {
        Self {
            content: row.content.clone(),
            status: row.status.to_string(),
        }
    }
}

impl Record {
    /// The trust map this session had, or `None` if it did not record one.
    ///
    /// An integrity this build does not recognise reads as untrusted, the safe direction, as
    /// [`bravebot_agent::conversation::Snapshot`] already does for the context. A hand-edited or
    /// newer-than-this-build record therefore resumes with less trust rather than more.
    pub fn trust_map(&self) -> Option<TrustStore> {
        let rules = self.trust.as_ref()?;
        let mut trust = TrustStore::new();
        for rule in rules {
            if rule.integrity == TRUSTED {
                trust.trust(&rule.path);
            } else {
                trust.distrust(&rule.path);
            }
        }
        Some(trust)
    }

    /// The programs this session's user vouched for.
    ///
    /// An empty list where a record predates this being kept, which is the safe direction: every
    /// run asks, rather than a resumed session inheriting a permission nobody recorded.
    pub fn trusted_programs(&self) -> TrustedPrograms {
        TrustedPrograms::from_iter(
            self.programs
                .iter()
                .map(|c| bravebot_core::programs::Command::new(c.program.clone(), c.args.clone())),
        )
    }

    /// Open again the directories this session added, and say which could not be opened.
    ///
    /// The map is only half of what `/add-dir` granted, and it is the half that is no use alone:
    /// restoring the rule without the directory leaves every path under it refused for escaping
    /// the workspace, with nothing on screen to say why. So the reachability is restored beside
    /// the rule that vouches for it.
    ///
    /// A directory that has since been moved or deleted comes back as a line for the user rather
    /// than as silence, because the rule about it does come back and a rule about files nothing
    /// can open is the failure this exists to rule out.
    pub fn reopen_added_directories(&self, workspace: &mut Workspace) -> Vec<String> {
        self.directories
            .iter()
            .filter_map(|directory| match workspace.add_directory(directory) {
                Ok(_) => None,
                Err(error) => Some(t!(
                    session_reopen_failed,
                    directory = directory,
                    problem = error
                )),
            })
            .collect()
    }

    /// The task lists this session kept, shaped for a screen.
    ///
    /// A status this build does not recognise parses as outstanding work, which is
    /// [`Status::parse`]'s own rule: an item nobody can classify is the one reading that cannot
    /// quietly hide something.
    pub fn todo_rows(&self) -> BTreeMap<usize, Vec<Row>> {
        self.todos
            .iter()
            .map(|(turn, tasks)| {
                let items = tasks
                    .iter()
                    .map(|task| Item::new(task.content.clone(), Status::parse(&task.status)))
                    .collect();
                (*turn, todo::rows(&List::new(items)))
            })
            .collect()
    }
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
    /// Whether this was a manifest run, which can be read but not continued.
    ///
    /// On the summary rather than left to the record, because the picker has to say so on the
    /// row: finding out only after selecting one is finding out too late.
    pub manifest: bool,
}

/// What a session amounts to at the moment it is written down.
///
/// A value rather than a row of arguments, because everything a resume needs restored ends up
/// here and the list was growing one parameter at a time.
#[derive(Debug, Clone, Copy)]
pub struct Standing<'a> {
    pub conversation: &'a Snapshot,
    pub turns: usize,
    pub tokens: u64,
    /// What each turn cost, by turn number.
    pub spend: &'a BTreeMap<usize, u64>,
    /// Where each turn's wall clock went, by turn number.
    pub timing: &'a BTreeMap<usize, bravebot_agent::timing::Timing>,
    /// The model the server reported answering with, or `None` before a turn reached one.
    pub model: Option<&'a str>,
    pub todos: &'a BTreeMap<usize, Vec<Row>>,
    pub trust: &'a TrustStore,
    pub programs: &'a TrustedPrograms,
    pub directories: &'a [PathBuf],
    /// What a manifest run produced. `None` for a turn session, which is every session the
    /// interactive interface writes.
    pub manifest: Option<&'a StoredManifest>,
}

/// A live session, holding where to write and what has been written.
#[derive(Debug, Clone)]
pub struct Handle {
    id: String,
    project: PathBuf,
    started: u64,
    branch: Option<String>,
    title: String,
    /// Whether a record for this id is on disk yet.
    ///
    /// An id exists from the first moment, but a session that was opened and abandoned leaves
    /// nothing to pick up again, and offering to resume one is offering something that does not
    /// work.
    wrote: bool,
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
            wrote: false,
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
            // The record it came from is the one being written back to.
            wrote: true,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// The id to hand somebody wanting this session back, or `None` if there is nothing to hand.
    ///
    /// A session with no record on disk cannot be resumed, so its id is not worth printing: a
    /// command that would answer "no session by that name" is worse than saying nothing.
    pub fn resumable(&self) -> Option<&str> {
        self.wrote.then_some(self.id.as_str())
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Call the session something the user chose.
    ///
    /// Takes effect at once rather than at the next turn, by rewriting the record where there is
    /// one: a session renamed and then left alone should be findable under its new name, and one
    /// renamed before its first turn has no record to rewrite yet, so the name waits on the handle
    /// and `save` writes it.
    ///
    /// The name is trimmed and cut to the length a derived title gets, since it goes in the same
    /// column of the same list. An empty name is refused, which the caller reports: silently
    /// keeping the old one would look like the rename worked.
    pub fn rename(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        self.title = title_from(name);
        self.rewrite_title();
        true
    }

    /// Put the current title into the record on disk, if the session has one yet.
    ///
    /// Read, amended and written rather than rebuilt, because everything else in the record belongs
    /// to the turns that produced it and this knows none of it.
    fn rewrite_title(&self) {
        let Some(directory) = self.directory() else {
            return;
        };
        let path = directory.join(format!("{}.json", self.id));
        let Some(mut record) = read(&path) else {
            return;
        };
        record.title = self.title.clone();
        record.updated = now();

        let Ok(body) = serde_json::to_vec_pretty(&record) else {
            return;
        };
        // Beside and renamed, as `save` does, so an interrupted rename leaves the record it had.
        let temporary = directory.join(format!("{}.tmp", self.id));
        if std::fs::write(&temporary, body).is_ok() {
            let _ = std::fs::rename(&temporary, path);
        }
    }

    /// Write the session down as it now stands.
    ///
    /// Called after each turn rather than at the end, because the end may never come: a session
    /// that was killed, or whose machine slept and never woke, is exactly the one worth
    /// resuming.
    pub fn save(&mut self, first_prompt: &str, standing: Standing<'_>) {
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
            turns: standing.turns,
            tokens: standing.tokens,
            model: standing.model.map(str::to_string),
            spend: standing.spend.clone(),
            timing: standing.timing.clone(),
            todos: standing
                .todos
                .iter()
                .map(|(turn, rows)| (*turn, rows.iter().map(StoredTask::of).collect()))
                .collect(),
            trust: Some(
                standing
                    .trust
                    .rules()
                    .map(|(path, integrity)| StoredRule {
                        path: path.to_string(),
                        integrity: match integrity {
                            Integrity::Trusted => TRUSTED,
                            Integrity::Untrusted => UNTRUSTED,
                        }
                        .to_string(),
                    })
                    .collect(),
            ),
            programs: standing
                .programs
                .iter()
                .map(|c| StoredCommand {
                    program: c.program.clone(),
                    args: c.args.clone(),
                })
                .collect(),
            directories: standing
                .directories
                .iter()
                .map(|d| d.display().to_string())
                .collect(),
            build: Some(crate::BUILD.to_string()),
            conversation: standing.conversation.clone(),
            manifest: standing.manifest.cloned(),
        };

        let Ok(body) = serde_json::to_vec_pretty(&record) else {
            return;
        };

        // Written beside and renamed, so a session killed mid-write leaves the last good record
        // rather than half of a new one.
        let temporary = directory.join(format!("{}.tmp", self.id));
        if std::fs::write(&temporary, body).is_ok()
            && std::fs::rename(&temporary, directory.join(format!("{}.json", self.id))).is_ok()
        {
            self.wrote = true;
        }
    }

    /// Append what one turn's gates decided.
    ///
    /// One JSON object per line, so the file can be grown a turn at a time and read with
    /// ordinary tools. What goes in it is gate names, labels and paths: the audit says what was
    /// allowed and why, never what the content was.
    pub fn append_audit(&self, turn: usize, events: &[crate::audit::Stamped]) {
        let Some(directory) = self.directory() else {
            return;
        };

        let mut body = String::new();
        for stamped in events {
            // The event's own time, not this moment. A turn is written down once, at the end, so
            // stamping here made every event in it share a second and left the trail unable to
            // say which came first or how long anything took.
            let line = serde_json::json!({
                "at": stamped.at,
                "turn": turn,
                "event": crate::audit::as_json(&stamped.event),
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
                manifest: record.manifest.is_some(),
            })
        })
        .collect();

    newest_first(&mut summaries);
    summaries
}

/// Order a list so the most recently written session comes first.
///
/// The picker offers the top entry, so the direction is the behaviour rather than a detail of
/// how the list is built. Named and separate because a reversed comparator is silent: the list
/// still renders, just with the session someone is least likely to want at the top.
fn newest_first(summaries: &mut [Summary]) {
    summaries.sort_by_key(|s| std::cmp::Reverse(s.updated));
}

/// Read one session back, by the id the list gave.
pub fn load(project: &Path, id: &str) -> Option<Record> {
    let directory = project_directory(project)?;
    read(&directory.join(format!("{id}.json")))
}

/// What a resumed session shows beneath each turn, by turn number.
///
/// Two files behind one type: the plan comes out of the record and the trail out of the audit
/// beside it. The transcript wants them together, since both hang off the same entry.
#[derive(Debug, Default)]
pub struct Recalled {
    pub trails: BTreeMap<usize, Vec<crate::audit::TrailLine>>,
    pub todos: BTreeMap<usize, Vec<Row>>,
}

/// Everything a resumed transcript needs beyond the conversation itself.
pub fn recall(project: &Path, record: &Record) -> Recalled {
    Recalled {
        trails: audit_of(project, &record.id),
        todos: record.todo_rows(),
    }
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

/// What to say about a session being picked up somewhere other than where it ran.
///
/// `None` when nothing moved, which is the ordinary case and should cost no line.
///
/// Worth saying at all because the transcript is about to be shown as though the work were still
/// in front of the user, and half of it may no longer be: a session that was editing a feature
/// branch, resumed on main, will be asked to carry on with changes that are not there. The record
/// knew and said nothing, since [`Handle::resuming`] replaces the branch with the current one.
pub fn branch_note(was: Option<&str>, now: Option<&str>) -> Option<String> {
    if was == now {
        return None;
    }
    Some(match (was, now) {
        (Some(was), Some(now)) => t!(session_branch_moved, was = was, now = now),
        (Some(was), None) => t!(session_branch_gone, was = was),
        (None, Some(now)) => t!(session_branch_new, now = now),
        (None, None) => unreachable!("equal cases returned above"),
    })
}

/// The branch checked out in `directory`, where it is a git checkout at all.
///
/// Read out of the files rather than by running git: it is one line, and this is a label on a
/// list entry. A detached head has no branch name, which is reported as none rather than as the
/// commit it happens to be on.
/// What to say when the build that recorded a session is not the one resuming it.
///
/// The same kind of caveat as [`branch_note`], and for the same reason: what the transcript above
/// describes was done by something other than what is about to carry on. Silent for a record
/// with no build written down, which is one from before this was kept and has nothing to compare.
pub fn build_note(was: Option<&str>, now: &str) -> Option<String> {
    let was = was?;
    (was != now).then(|| t!(session_build_differs, was = was, now = now))
}

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
    bravebot_agent::report::how_long_ago(std::time::Duration::from_secs(seconds))
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

/// A session's name: a version 4 UUID.
///
/// Opaque on purpose. It used to be the time and the process id, which sorted by age and read as
/// two facts about the machine that made it, neither of which is anybody's business once the name
/// is printed on a screen and pasted into a command. Nothing orders sessions by id: the list is
/// sorted on what the record says it was last written.
///
/// Random rather than counted, so two of them cannot collide however many processes are running
/// and whatever the clock does.
fn new_id() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    // The version and variant bits, so what comes out is a well-formed UUID rather than thirty-two
    // hex characters that merely look like one.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    let mut id = String::with_capacity(36);
    for (at, byte) in bytes.iter().enumerate() {
        if matches!(at, 4 | 6 | 8 | 10) {
            id.push('-');
        }
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name is printed on the way out and pasted into a command, so it has to be the shape a
    /// person recognises as an id and nothing else. It used to be the time and the process id.
    #[test]
    fn a_session_is_named_by_a_uuid() {
        let id = new_id();

        assert_eq!(id.len(), 36, "{id}");
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12],
            "{id}"
        );
        assert!(
            id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()),
            "{id} is not hexadecimal"
        );
        assert!(parts[2].starts_with('4'), "not version 4: {id}");
        assert!(
            parts[3].starts_with(['8', '9', 'a', 'b']),
            "not the RFC 4122 variant: {id}"
        );
    }

    /// Two sessions must never be one session. Counted or clocked ids collided when two processes
    /// started in the same second, which is exactly what happens when somebody opens two windows.
    #[test]
    fn no_two_sessions_are_given_the_same_name() {
        let names: std::collections::BTreeSet<String> = (0..64).map(|_| new_id()).collect();
        assert_eq!(names.len(), 64, "an id came up twice");
    }

    #[test]
    fn a_working_directory_becomes_one_readable_segment() {
        let key = key_for(Path::new("/Users/someone/projects/bravebot"));
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

    /// Every session written before timing was kept has a record without it, and refusing to parse
    /// one would make an upgrade look like a lost session. It reads as no breakdown, which is not
    /// the same as a session that took no time: the turn count and the token total are still there.
    #[test]
    fn a_record_written_before_timing_was_kept_still_loads() {
        let body = serde_json::to_string(&serde_json::json!({
            "id": "1-2",
            "directory": "/tmp/x",
            "title": "a session",
            "started": 1,
            "updated": 1,
            "turns": 2,
            "tokens": 3_400,
            "conversation": {
                "messages": [],
                "context": "trusted",
                "references": 0,
                "archive": [],
                "measured": 0,
            },
        }))
        .expect("serialises");

        let record: Record = serde_json::from_str(&body).expect("an older record still parses");
        assert!(record.timing.is_empty(), "a breakdown was invented");
        assert_eq!(record.tokens, 3_400, "the figures that were kept were lost");
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

    /// A scratch checkout, so the test says what the code reads rather than what this machine
    /// happens to have checked out.
    fn fake_checkout(name: &str, head: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bravebot-sessions-{name}"));
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

    /// A record from before the list was kept vouches for nothing, which is the safe direction:
    /// every run asks, rather than a resumed session inheriting a permission nobody recorded.
    #[test]
    fn a_record_without_a_program_list_vouches_for_nothing() {
        let record = a_record();
        assert!(record.trusted_programs().is_empty());
    }

    /// What was written down comes back, by resolved path, so a resumed session stops asking about
    /// exactly the programs its own user vouched for.
    #[test]
    fn the_programs_a_session_vouched_for_come_back() {
        let mut record = a_record();
        record.programs = vec![
            StoredCommand {
                program: "/usr/bin/git".to_string(),
                args: vec!["log".to_string()],
            },
            StoredCommand {
                program: "/bin/ls".to_string(),
                args: Vec::new(),
            },
        ];
        let vouched = record.trusted_programs();
        assert!(vouched.contains("/usr/bin/git", &["log".to_string()]));
        assert!(vouched.contains("/bin/ls", &[]));
        assert!(
            !vouched.contains("/usr/bin/git", &["push".to_string()]),
            "a record vouched for a command it never named"
        );
        assert!(
            !vouched.contains("/opt/homebrew/bin/git", &["log".to_string()]),
            "a record vouched for a binary it never named"
        );
    }

    /// A record from before the map was kept must be asked about, not read as a map that trusts
    /// nothing. The two look the same in the end and are answered differently: nothing recorded
    /// is a question, and an empty map is an answer.
    #[test]
    fn a_record_that_predates_the_map_has_none_rather_than_an_empty_one() {
        let older = serde_json::json!({
            "id": "1-2",
            "directory": "/tmp/x",
            "title": "older",
            "started": 1,
            "updated": 1,
            "conversation": {"messages": [], "context": "trusted"},
        });
        let record: Record = serde_json::from_value(older).expect("an older record still loads");
        assert!(record.trust_map().is_none());
    }

    /// Whatever a record says that this build does not recognise, the answer is untrusted. A
    /// hand edit or a newer build's word lands in the safe direction, as everything else does.
    #[test]
    fn an_unrecognised_integrity_in_a_record_reads_as_untrusted() {
        for word in ["", "TRUSTED", "trusted-ish", "yes"] {
            let mut record = a_record();
            record.trust = Some(vec![StoredRule {
                path: ".".to_string(),
                integrity: word.to_string(),
            }]);
            let map = record.trust_map().expect("a map was recorded");
            assert!(
                !map.is_trusted("src/main.rs"),
                "{word:?} was read as trusted"
            );
        }
    }

    /// The rule the whole map turns on has to survive being written down: a path a write marked
    /// untrusted, inside a tree the user vouched for, stays untrusted when the session resumes.
    #[test]
    fn a_distrusted_path_inside_a_trusted_tree_survives_the_record() {
        let mut record = a_record();
        record.trust = Some(vec![
            StoredRule {
                path: String::new(),
                integrity: "trusted".to_string(),
            },
            StoredRule {
                path: "src/fetched.json".to_string(),
                integrity: "untrusted".to_string(),
            },
        ]);

        let map = record.trust_map().expect("a map was recorded");
        assert!(map.is_trusted("src/main.rs"));
        assert!(!map.is_trusted("src/fetched.json"));
    }

    /// The picker offers the top entry, so a reversed comparator would silently hand someone
    /// the session they last touched a month ago. Nothing else in the suite pins the direction.
    #[test]
    fn a_list_puts_the_most_recently_written_session_first() {
        fn at(updated: u64) -> Summary {
            Summary {
                id: format!("s-{updated}"),
                title: "a session".to_string(),
                branch: None,
                updated,
                bytes: 0,
                manifest: false,
            }
        }

        let mut summaries = vec![at(10), at(30), at(20)];
        newest_first(&mut summaries);

        assert_eq!(
            summaries.iter().map(|s| s.updated).collect::<Vec<_>>(),
            vec![30, 20, 10]
        );
    }

    /// A transcript is read after the fact, and the first question about a strange one is
    /// whether the code that produced it is the code in front of you. Inferring that from the
    /// transcript's own symptoms is guesswork at the moment guesswork is worth least.
    #[test]
    fn a_record_says_which_build_wrote_it() {
        assert!(
            crate::BUILD.starts_with(env!("CARGO_PKG_VERSION")),
            "the build stamp does not name the version: {}",
            crate::BUILD
        );
    }

    /// Resuming on different code is a caveat on the transcript above it, exactly as resuming on
    /// a different branch is.
    #[test]
    fn a_session_recorded_by_another_build_says_so() {
        assert_eq!(build_note(Some("0.1.0 (aaaaaaa)"), "0.1.0 (aaaaaaa)"), None);
        let note = build_note(Some("0.1.0 (aaaaaaa)"), "0.1.0 (bbbbbbb)")
            .expect("a different build is worth saying");
        assert!(
            note.contains("aaaaaaa") && note.contains("bbbbbbb"),
            "{note}"
        );
        // Nothing recorded is nothing to compare, rather than something to remark on.
        assert_eq!(build_note(None, "0.1.0 (bbbbbbb)"), None);
    }

    fn a_record() -> Record {
        Record {
            id: "1-2".to_string(),
            directory: "/tmp/x".to_string(),
            branch: None,
            title: "a session".to_string(),
            started: 1,
            updated: 1,
            turns: 0,
            tokens: 0,
            model: None,
            spend: BTreeMap::new(),
            timing: BTreeMap::new(),
            todos: BTreeMap::new(),
            trust: None,
            programs: Vec::new(),
            directories: Vec::new(),
            build: None,
            conversation: Snapshot {
                messages: Vec::new(),
                context: "trusted".to_string(),
                references: 0,
                archive: Vec::new(),
                measured: 0,
            },
            manifest: None,
        }
    }

    /// The ordinary case, which must cost no line: a session picked up where it was left.
    #[test]
    fn resuming_on_the_same_branch_is_not_worth_saying() {
        assert_eq!(branch_note(Some("main"), Some("main")), None);
        assert_eq!(branch_note(None, None), None);
    }

    /// A session that was editing a feature branch, resumed on main, is about to be asked to
    /// carry on with changes that are not there. Both names go in the line, since which one is
    /// wanted is the user's decision and they need to see both to make it.
    #[test]
    fn resuming_on_another_branch_says_which_one_it_ran_on() {
        let note = branch_note(Some("feature-x"), Some("main")).expect("a note");
        assert!(note.contains("feature-x"), "{note}");
        assert!(note.contains("main"), "{note}");
    }

    /// A detached head has no name to print, and the move is still worth reporting: the branch
    /// the work was on is not the thing checked out.
    #[test]
    fn moving_on_or_off_a_branch_is_still_a_move() {
        let note = branch_note(Some("main"), None).expect("a note");
        assert!(note.contains("main"), "{note}");
        assert!(note.contains("not on a branch"), "{note}");

        let note = branch_note(None, Some("main")).expect("a note");
        assert!(note.contains("main"), "{note}");
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
        let root = std::env::temp_dir().join("bravebot-sessions-not-a-checkout");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create scratch");
        assert_eq!(branch_of(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
