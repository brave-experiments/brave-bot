//! Label-aware filesystem tools.
//!
//! Both operations split into a **routing** part and a **content** part, and the split
//! is what the policy gate checks:
//!
//! - `read(path)`: the path is routing. It must be `(T,pub)`, so untrusted content can
//!   never choose which file is read.
//! - `write(path, contents)`: the path is routing, the contents are content. Untrusted
//!   text may be written *into* a file it could not choose.
//!
//! Paths are also confined to a workspace root. That is a second, independent check:
//! the routing label stops content from *supplying* a path, while confinement stops a
//! trusted-but-wrong path from escaping the project.

use base64::Engine;
use bravebot_core::capability::Capability;
use bravebot_core::event::{Role, Sink};
use bravebot_core::label::Label;
use bravebot_core::policy::{Denial, Policy};
use bravebot_core::value::Labelled;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The most an attachment may weigh.
///
/// The whole thing goes into the request and is re-sent on every later round, so this bounds a
/// growing cost rather than a single one. Generous enough for a screenshot or a scanned page,
/// which is what people attach.
pub const MAX_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub enum WorkspaceError {
    /// The policy refused the operation.
    Denied(Denial),
    /// The path resolved outside the workspace root.
    Escapes { path: String },
    /// The path was not usable as a relative workspace path.
    Invalid { path: String, reason: &'static str },
    /// The operation failed on disk.
    Io { path: String, detail: String },
    /// The file changed after it was read, so the approved change no longer applies.
    Stale { path: String },
    /// The file is not text, so there is nothing useful to return.
    Binary { path: String },
    /// The attachment is larger than a request should carry.
    TooLarge { path: String, limit: usize },
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied(d) => write!(f, "{d}"),
            Self::Escapes { path } => write!(
                f,
                "'{path}' resolves outside the workspace; refusing to touch it"
            ),
            Self::Invalid { path, reason } => write!(f, "'{path}' is not usable: {reason}"),
            Self::Io { path, detail } => write!(f, "'{path}': {detail}"),
            Self::Stale { path } => write!(
                f,
                "'{path}' changed after it was read; read it again before editing"
            ),
            Self::Binary { path } => {
                write!(f, "'{path}' is a binary file, so it cannot be read as text")
            }
            Self::TooLarge { path, limit } => write!(
                f,
                "'{path}' is larger than the {} MiB an attachment may be",
                limit / (1024 * 1024)
            ),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl From<Denial> for WorkspaceError {
    fn from(value: Denial) -> Self {
        Self::Denied(value)
    }
}

/// How far a read of a file's text may reach.
///
/// Named rather than passed as a flag because the two are not variants of a setting: everything
/// here is confined to the workspace, and the one exception exists because a person's own gesture
/// named the file. See [`Workspace::read_dropped_text`] for why that is the boundary.
#[derive(Debug, Clone, Copy)]
enum Reach {
    /// The workspace and the directories the user added, and nowhere else.
    Confined,
    /// Wherever the drop pointed.
    Dropped,
}

/// The directories file operations are confined to.
///
/// One primary root, which every relative path is resolved against, plus any the user added by
/// name with `/add-dir`. An added directory is reachable only by its absolute path, so the two
/// never overlap: a relative path always means the project, whatever else is open.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    /// Absolute directories the user named, each canonical.
    ///
    /// Kept apart from `root` rather than being a list of equals, because the primary root is what
    /// relative paths mean, what the session record is keyed on, and where `AGENTS.md` is looked
    /// for. Making it one root among many would make all three ambiguous.
    added: Vec<PathBuf>,
    /// How many files a search may walk. [`MAX_SEARCH_FILES`] unless a caller lowered it.
    ///
    /// A field rather than a constant so a test can reach the cap without writing a hundred
    /// thousand files, and so a host on a slow filesystem can say so.
    search_files: usize,
    /// What the files this turn has written held before it wrote to them.
    ///
    /// Behind a lock and a handle because a workspace is cloned into the turn that uses it, and a
    /// rewind has to see what that copy wrote. Nothing here is read: the bytes are carried back to
    /// the path they came from and never inspected.
    backups: Arc<Mutex<Vec<Backup>>>,
}

/// What a path held before a turn wrote to it.
///
/// Carried, never read. The driver hands the bytes back to the path they came from and has no
/// business looking at them on the way.
#[derive(Debug, Clone)]
pub struct Backup {
    /// The file, resolved to an absolute path as the write resolved it.
    pub path: PathBuf,
    /// What was there, or `None` where the write created the file.
    pub was: Option<Vec<u8>>,
}

/// What changed when the working directory moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// The new primary root, canonical.
    pub root: PathBuf,
    /// Directories that were open and are not any more, in the order they closed, the one left
    /// behind first. Empty is possible: moving into a directory that was already open closes
    /// nothing but the root it replaces.
    pub closed: Vec<PathBuf>,
}

/// Whether two directories are the same tree, or one holds the other.
///
/// Component-wise, so `/work/srcfoo` is not taken to be inside `/work/src`. A test on the
/// characters would close a directory that merely shares a prefix with the new root's name.
fn overlaps(one: &Path, other: &Path) -> bool {
    one.starts_with(other) || other.starts_with(one)
}

impl Workspace {
    /// Create a workspace at `root`, which must exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, WorkspaceError> {
        let root = root.into();
        let canonical = root.canonicalize().map_err(|e| WorkspaceError::Io {
            path: root.display().to_string(),
            detail: e.to_string(),
        })?;
        Ok(Self {
            root: canonical,
            added: Vec::new(),
            search_files: MAX_SEARCH_FILES,
            backups: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Lower how many files a search may walk.
    ///
    /// Only ever lowered in practice: the default is chosen to be past what any tree a person
    /// works in holds, and raising it trades a bounded search for an unbounded one.
    #[must_use]
    pub fn with_search_limit(mut self, files: usize) -> Self {
        self.search_files = files;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether an absolute path is one this workspace may touch.
    ///
    /// For a destination something else opens, which is what a redirection in a command line is:
    /// the plan names the file and the run opens it directly, so the confinement every other write
    /// goes through has to be applied to the path rather than to a call through here.
    ///
    /// The file need not exist yet, so the nearest ancestor that does is what is canonicalised.
    /// That is also what catches a symlink pointing out of the tree, since the link is resolved
    /// before the comparison rather than after.
    pub fn confines(&self, path: &Path) -> Result<(), WorkspaceError> {
        let escapes = || WorkspaceError::Escapes {
            path: path.display().to_string(),
        };
        let mut existing = path;
        let mut trailing = PathBuf::new();
        while !existing.exists() {
            let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
                return Err(escapes());
            };
            trailing = Path::new(name).join(&trailing);
            existing = parent;
        }
        let canonical = existing
            .canonicalize()
            .map_err(|_| escapes())?
            .join(trailing);
        if canonical.starts_with(&self.root)
            || self.added.iter().any(|dir| canonical.starts_with(dir))
        {
            return Ok(());
        }
        Err(escapes())
    }

    /// Also allow paths inside `directory`, which must exist.
    ///
    /// Returns the canonical path, which is what the caller records trust against and shows the
    /// user: the name they typed may be a symlink or contain `..`, and the rule has to be about the
    /// directory that was actually opened.
    ///
    /// A directory already inside the primary root is refused. It is reachable by its relative path
    /// already, and admitting it would give one file two spellings, one governed by the project's
    /// trust rules and one by its own.
    pub fn add_directory(&mut self, directory: &str) -> Result<PathBuf, WorkspaceError> {
        let candidate = Path::new(directory);
        if !candidate.is_absolute() {
            return Err(WorkspaceError::Invalid {
                path: directory.to_string(),
                reason: "must be an absolute path",
            });
        }

        let canonical = candidate.canonicalize().map_err(|e| WorkspaceError::Io {
            path: directory.to_string(),
            detail: e.to_string(),
        })?;

        if !canonical.is_dir() {
            return Err(WorkspaceError::Invalid {
                path: directory.to_string(),
                reason: "must be a directory",
            });
        }

        if canonical.starts_with(&self.root) {
            return Err(WorkspaceError::Invalid {
                path: directory.to_string(),
                reason: "is already inside the workspace, so it can be named relatively",
            });
        }

        if !self.added.contains(&canonical) {
            self.added.push(canonical.clone());
        }
        Ok(canonical)
    }

    /// The directories added by name, in the order they were added.
    pub fn added_directories(&self) -> &[PathBuf] {
        &self.added
    }

    /// Work in `directory` from now on, which must exist.
    ///
    /// The primary root is what a relative path means, so moving it moves the whole of what this
    /// workspace is about. Returns the canonical path, and the directories that were open and are
    /// no longer, which the caller has to tell the user about: reach that goes quietly is reach
    /// somebody will discover by being refused a file they could read a minute ago.
    ///
    /// **Nothing may overlap the new root.** The directory left behind is closed, and so is any
    /// added directory inside the new root or containing it. That is not tidiness: a file inside
    /// two open directories has two spellings, one relative and one absolute, and the two are
    /// separate namespaces in the trust map. A tree reachable by both spellings could be read
    /// under whichever rule was more permissive, which is the one thing keeping the namespaces
    /// apart exists to prevent. An added directory that overlaps nothing is left open, since the
    /// user opened it by name and moving elsewhere does not withdraw that.
    pub fn change_root(&mut self, directory: &str) -> Result<Moved, WorkspaceError> {
        let candidate = Path::new(directory);
        if !candidate.is_absolute() {
            return Err(WorkspaceError::Invalid {
                path: directory.to_string(),
                reason: "must be an absolute path",
            });
        }

        let canonical = candidate.canonicalize().map_err(|e| WorkspaceError::Io {
            path: directory.to_string(),
            detail: e.to_string(),
        })?;

        if !canonical.is_dir() {
            return Err(WorkspaceError::Invalid {
                path: directory.to_string(),
                reason: "must be a directory",
            });
        }

        if canonical == self.root {
            return Err(WorkspaceError::Invalid {
                path: directory.to_string(),
                reason: "is already the working directory",
            });
        }

        // The old root among them: it is a directory that was open, and after this it is not.
        let mut closed = vec![std::mem::replace(&mut self.root, canonical.clone())];
        let (overlapping, kept) = std::mem::take(&mut self.added)
            .into_iter()
            .partition(|open| overlaps(open, &canonical));
        self.added = kept;
        closed.extend(overlapping);
        // Whatever the new root is now reachable as itself, so it did not close.
        closed.retain(|open| *open != canonical);

        Ok(Moved {
            root: canonical,
            closed,
        })
    }

    /// Close every directory added by name, leaving only the primary root.
    ///
    /// For starting over inside one process: opening a directory is a grant, so it goes when the
    /// grants do. Leaving them open while the trust map that vouched for them was discarded would
    /// leave a tree reachable that nobody had vouched for.
    pub fn close_added_directories(&mut self) {
        self.added.clear();
    }

    /// Resolve a path against the workspace.
    ///
    /// A relative path always means the primary root. An absolute path is legal only inside a
    /// directory the user added by name, and is refused otherwise: an absolute path was refused
    /// outright before `/add-dir` existed, and naming a directory is what makes one reachable.
    ///
    /// Rejects any `..` component. `..` is rejected before touching the filesystem rather than by
    /// canonicalising afterwards, because the target of a write may not exist yet, and a check
    /// that only works for existing files would leave writes unprotected.
    fn resolve(&self, relative: &str) -> Result<PathBuf, WorkspaceError> {
        let candidate = Path::new(relative);

        if candidate.is_absolute() {
            return self.resolve_added(candidate, relative);
        }

        for component in candidate.components() {
            match component {
                Component::ParentDir => {
                    return Err(WorkspaceError::Escapes {
                        path: relative.to_string(),
                    });
                }
                Component::Prefix(_) | Component::RootDir => {
                    return Err(WorkspaceError::Invalid {
                        path: relative.to_string(),
                        reason: "must not name a root or drive",
                    });
                }
                Component::CurDir | Component::Normal(_) => {}
            }
        }

        let joined = self.root.join(candidate);

        // For paths that already exist, confirm the resolved location is still inside
        // the root. This is what catches a symlink pointing out of the workspace.
        if let Ok(canonical) = joined.canonicalize() {
            if !canonical.starts_with(&self.root) {
                return Err(WorkspaceError::Escapes {
                    path: relative.to_string(),
                });
            }
            return Ok(canonical);
        }

        Ok(joined)
    }

    /// Resolve an absolute path, which is legal only inside a directory the user added.
    ///
    /// The containment test is against the canonical path where one exists, so a symlink inside an
    /// added directory pointing elsewhere is refused exactly as one in the primary root is. Where
    /// the file does not exist yet, the lexical path is tested instead, which is what lets a write
    /// create a file; `..` is rejected first, so there is nothing lexical containment can miss.
    /// Where an attachment's bytes are, which may be anywhere the user pointed at.
    ///
    /// Relative paths resolve against the root like everything else. An absolute one is taken as
    /// given, because a person dropped it: see [`Workspace::read_attachment`] for why that is the
    /// boundary rather than a directory check, and why nothing else here resolves this way.
    ///
    /// A directory is refused. Dropping one is a plausible slip and reading it would otherwise
    /// fail further down with a message about bytes.
    fn resolve_attachment(&self, named: &str) -> Result<PathBuf, WorkspaceError> {
        let candidate = Path::new(named);
        let resolved = if candidate.is_absolute() {
            candidate.canonicalize().map_err(|e| WorkspaceError::Io {
                path: named.to_string(),
                detail: e.to_string(),
            })?
        } else {
            self.resolve(named)?
        };

        if resolved.is_dir() {
            return Err(WorkspaceError::Invalid {
                path: named.to_string(),
                reason: "a directory cannot be attached",
            });
        }

        Ok(resolved)
    }

    fn resolve_added(&self, candidate: &Path, named: &str) -> Result<PathBuf, WorkspaceError> {
        if candidate
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(WorkspaceError::Escapes {
                path: named.to_string(),
            });
        }

        let inside = |path: &Path| self.added.iter().any(|dir| path.starts_with(dir));

        if let Ok(canonical) = candidate.canonicalize() {
            if !inside(&canonical) {
                return Err(WorkspaceError::Escapes {
                    path: named.to_string(),
                });
            }
            return Ok(canonical);
        }

        if !inside(candidate) {
            return Err(WorkspaceError::Escapes {
                path: named.to_string(),
            });
        }
        Ok(candidate.to_path_buf())
    }

    /// Read a file in full. The path is checked as routing, so it must be `(T,pub)`.
    ///
    /// The contents are private, being the user's data, and their integrity comes from
    /// the trust map: a file read out of a trusted directory is trusted, anything else is not.
    ///
    /// Deliberately uncapped, because the callers that need it need all of it: an edit
    /// replaces text in the whole file and compares against it to detect a concurrent
    /// change, so a truncated read here would write back a shortened file. The tool the
    /// model calls uses [`Workspace::read_page`] instead.
    pub fn read<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        path: &Labelled<String>,
    ) -> Result<Labelled<String>, WorkspaceError> {
        self.read_text(policy, path, Reach::Confined)
    }

    /// Read a text file a person dropped on the window, wherever on the disk it sits.
    ///
    /// [`Workspace::read`] in every respect but path confinement, and it gives that up for the
    /// same reason [`Workspace::read_attachment`] does: a drop nearly always comes from
    /// `~/Downloads` or `~/Desktop`, so confining it would refuse the case the gesture exists for.
    /// A dropped `.md` is a dropped file exactly as a dropped `.png` is; that one becomes context
    /// rather than bytes is a fact about its type, not about where it may live.
    ///
    /// What makes reaching out sound is the same thing there too: the path is precommitted into
    /// routing before the turn starts, from a gesture a person made, and the routing gate refuses
    /// anything that is not `(T,pub)`. No tool adds one, so nothing a model says can reach this,
    /// and no file's contents can either.
    ///
    /// [`Workspace::resolve`] is untouched, so reading, writing, editing, listing and searching
    /// stay confined exactly as they were: dropping a file lets that file be read, and grants
    /// nothing else anywhere.
    pub fn read_dropped_text<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        path: &Labelled<String>,
    ) -> Result<Labelled<String>, WorkspaceError> {
        self.read_text(policy, path, Reach::Dropped)
    }

    fn read_text<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        path: &Labelled<String>,
        reach: Reach,
    ) -> Result<Labelled<String>, WorkspaceError> {
        policy.before_capability(Capability::FileRead)?;
        policy.before_action("file_read", "path", Role::Routing, path)?;

        // Safe to read: before_action just proved this is (T,pub).
        let relative = path
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the path was not trusted",
            })?;

        // Not a decision taken from the bytes: the caller says which of the two reads this is,
        // and it says so from the shape of the gesture that produced the path.
        let resolved = match reach {
            Reach::Confined => self.resolve(&relative)?,
            Reach::Dropped => self.resolve_attachment(&relative)?,
        };
        let label = policy.observe_path(Capability::FileRead, &relative)?;

        let raw = std::fs::read(&resolved).map_err(|e| WorkspaceError::Io {
            path: relative.clone(),
            detail: e.to_string(),
        })?;

        // Named as binary rather than surfacing a decoding error. "stream did not contain
        // valid UTF-8" is an implementation detail that leaves a reader unable to tell a
        // binary file from a corrupt one.
        if looks_binary(&raw) {
            return Err(WorkspaceError::Binary { path: relative });
        }
        let contents = String::from_utf8(raw).map_err(|_| WorkspaceError::Binary {
            path: relative.clone(),
        })?;

        Ok(Labelled::new(contents, label))
    }

    /// Read a file the user attached, as a `data:` URI.
    ///
    /// The one read here that does not refuse a binary file, because a binary file is the point:
    /// an attachment is a screenshot or a PDF, and [`Workspace::read`] answers `Binary` for both.
    /// What comes back is still a `String`, so it needs no new content type in the kernel and
    /// carries a label like anything else.
    ///
    /// `media` is the type to name in the URI. It comes from the interface's own table of
    /// extensions, never from the file's bytes: sniffing content to decide how to describe it
    /// would be a decision derived from the very bytes nobody has vouched for.
    ///
    /// Every gate [`Workspace::read`] passes, in the same order and for the same reasons. The path
    /// is routing, so it must be `(T,pub)`; the contents are the user's data, so their integrity
    /// comes from the trust map.
    ///
    /// The one thing it does not share is path confinement, and that is deliberate. A drop hands
    /// over an absolute path, and it is nearly always `~/Downloads` or `~/Desktop`, so confining
    /// this to the workspace would refuse the case the feature exists for. What makes it sound is
    /// not a path check but where the path can have come from: an attachment is precommitted into
    /// routing before the turn starts, from a gesture a person made, and the routing gate above
    /// refuses anything that is not `(T,pub)`. There is no tool that adds one, so nothing a model
    /// says can reach this, and no file's contents can either.
    ///
    /// Scoped to this one function on purpose. [`Workspace::resolve`] is untouched, so reading,
    /// writing, editing, listing and searching stay confined exactly as they were: attaching a
    /// file lets that file be carried, and grants nothing else anywhere.
    ///
    /// Capped, unlike `read`. The whole file goes into the request and is re-sent on every later
    /// round, so an attachment nobody bounded is a cost multiplier that grows with the
    /// conversation. The cap is named in the error, since "it failed" leaves a user resizing an
    /// image by guesswork.
    pub fn read_attachment<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        path: &Labelled<String>,
        media: &str,
    ) -> Result<Labelled<String>, WorkspaceError> {
        policy.before_capability(Capability::FileRead)?;
        policy.before_action("file_read", "path", Role::Routing, path)?;

        // Safe to read: before_action just proved this is (T,pub).
        let relative = path
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the path was not trusted",
            })?;

        let resolved = self.resolve_attachment(&relative)?;
        let label = policy.observe_path(Capability::FileRead, &relative)?;

        let raw = std::fs::read(&resolved).map_err(|e| WorkspaceError::Io {
            path: relative.clone(),
            detail: e.to_string(),
        })?;

        if raw.len() > MAX_ATTACHMENT_BYTES {
            return Err(WorkspaceError::TooLarge {
                path: relative,
                limit: MAX_ATTACHMENT_BYTES,
            });
        }

        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);

        Ok(Labelled::new(
            format!("data:{media};base64,{encoded}"),
            label,
        ))
    }

    /// Read a bounded window of a file's lines, for the model.
    ///
    /// A whole file is the wrong unit for a conversation. Every turn re-sends the entire
    /// message history, so one large file read is paid for again on every subsequent
    /// round. An uncapped read is a cost multiplier, not just a big message.
    ///
    /// `offset` is 1-based to match how the lines are reported back, so a model can ask
    /// for the next page using the number it was just shown.
    pub fn read_page<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        path: &Labelled<String>,
        offset: usize,
        limit: usize,
    ) -> Result<Labelled<Page>, WorkspaceError> {
        policy.before_capability(Capability::FileRead)?;
        policy.before_action("file_read", "path", Role::Routing, path)?;

        let relative = path
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the path was not trusted",
            })?;

        let label = policy.observe_path(Capability::FileRead, &relative)?;
        Ok(Labelled::new(self.page(&relative, offset, limit)?, label))
    }

    /// One page of a file, with no gate of its own.
    ///
    /// Split out of [`Workspace::read_page`] for the deferred case, where the gates ran when the
    /// slot was reserved and the reading happens later, under
    /// [`bravebot_core::policy::Policy::materialise`], which observes the path again itself. What
    /// comes back is therefore unlabelled, and the kernel labels it: this returns the shape of a
    /// file and never decides what it means.
    pub fn page(
        &self,
        relative: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page, WorkspaceError> {
        let resolved = self.resolve(relative)?;

        let raw = std::fs::read(&resolved).map_err(|e| WorkspaceError::Io {
            path: relative.to_string(),
            detail: e.to_string(),
        })?;

        if looks_binary(&raw) {
            return Err(WorkspaceError::Binary {
                path: relative.to_string(),
            });
        }

        let contents = String::from_utf8(raw).map_err(|_| WorkspaceError::Binary {
            path: relative.to_string(),
        })?;

        let limit = limit.clamp(1, MAX_PAGE_LINES);
        let start = offset.saturating_sub(1);
        let total = contents.lines().count();

        let mut lines = Vec::new();
        let mut long_lines = 0usize;
        for line in contents.lines().skip(start).take(limit) {
            let mut text = line.to_string();
            if text.len() > MAX_LINE {
                truncate_on_char_boundary(&mut text, MAX_LINE);
                text.push_str(" … (line truncated)");
                long_lines += 1;
            }
            lines.push(text);
        }

        Ok(Page {
            lines,
            first_line: start + 1,
            total_lines: total,
            long_lines,
            ends_with_newline: contents.ends_with('\n'),
        })
    }

    /// What a deferred read must know before it can put off reading: how big the file is, and
    /// whether it is text at all.
    ///
    /// Both answers have to be had now rather than later. A path that names nothing is an error
    /// the planner is told about at the moment it asks, as it always was, and a binary file is
    /// refused the same way rather than becoming a reference to something no processor could
    /// use. The size is what the planner is told instead of a line count.
    ///
    /// The sniff reads the same prefix [`looks_binary`] would have seen, so it reaches the same
    /// verdict on the same file. A file that turns to rubbish after that prefix is caught when
    /// the bytes are actually read, which is where an eager read would have caught it too.
    pub fn survey(&self, relative: &str) -> Result<usize, WorkspaceError> {
        let resolved = self.resolve(relative)?;
        let io = |e: std::io::Error| WorkspaceError::Io {
            path: relative.to_string(),
            detail: e.to_string(),
        };

        let size = std::fs::metadata(&resolved).map_err(io)?.len();

        let mut head = vec![0u8; SNIFF_BYTES];
        let mut file = std::fs::File::open(&resolved).map_err(io)?;
        let read = read_up_to(&mut file, &mut head).map_err(io)?;
        if looks_binary(&head[..read]) {
            return Err(WorkspaceError::Binary {
                path: relative.to_string(),
            });
        }

        Ok(size.min(usize::MAX as u64) as usize)
    }

    /// The current contents of a workspace file, for showing a reviewer what a write
    /// would replace.
    ///
    /// Deliberately outside the policy gates: this is read on the user's behalf to
    /// populate a confirmation prompt, never handed to the model. `None` when the file
    /// does not exist or cannot be read as text.
    pub fn peek_for_review(&self, relative: &str) -> Option<String> {
        let resolved = self.resolve(relative).ok()?;
        std::fs::read_to_string(resolved).ok()
    }

    /// How long ago a workspace file was last written, for telling a reviewer what they are
    /// about to lose.
    ///
    /// Outside the gates for the same reason as [`Workspace::peek_for_review`]: it is read on
    /// the user's behalf for something shown to them, and never handed to the model. `None` when
    /// there is no such file, or when the filesystem will not say.
    pub fn age_of(&self, relative: &str) -> Option<std::time::Duration> {
        let resolved = self.resolve(relative).ok()?;
        let modified = std::fs::metadata(resolved).ok()?.modified().ok()?;
        // A file from the future, which a clock change or a copied timestamp can produce, is
        // reported as new rather than as an error.
        Some(modified.elapsed().unwrap_or_default())
    }

    /// Write an endorsed file, but only if it still holds `expected`.
    ///
    /// An edit is approved against contents that were read moments earlier. If the file
    /// changed in between, whether by another process or the user's editor, the approved diff no
    /// longer describes what would happen, so the write is refused rather than applied to
    /// text nobody reviewed.
    pub fn write_endorsed_if_unchanged<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        path: &Labelled<String>,
        contents: &Labelled<String>,
        expected: &str,
    ) -> Result<PathBuf, WorkspaceError> {
        // Checked before the gates so a stale edit is reported as staleness rather than
        // consuming the single-use endorsement.
        let relative = self.peek_relative(path)?;
        let current = self.peek_for_review(&relative).unwrap_or_default();
        if current != expected {
            return Err(WorkspaceError::Stale { path: relative });
        }

        self.write_endorsed(policy, path, contents)
    }

    /// The path as a plain string, for a check made on the user's behalf.
    ///
    /// Does not promote or trust anything: the value is used to look at the filesystem,
    /// never to decide that a write may proceed. The gates in [`Workspace::write_endorsed`]
    /// still run afterwards.
    fn peek_relative(&self, path: &Labelled<String>) -> Result<String, WorkspaceError> {
        let (value, _) = path.clone().into_parts_for_decoding();
        self.resolve(&value)?;
        Ok(value)
    }

    /// Write a file whose path was endorsed by a person.
    ///
    /// Distinct from [`Workspace::write`], which requires the path to be trusted
    /// beforehand. Here the path arrives untrusted from the model and the endorsement is
    /// what authorises it, so the gate consumes a single-use grant bound to this exact
    /// value. A grant for a different path does not match.
    pub fn write_endorsed<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        path: &Labelled<String>,
        contents: &Labelled<String>,
    ) -> Result<PathBuf, WorkspaceError> {
        policy.before_capability(Capability::FileWrite)?;

        // Promotion alone would not be enough for a write; the grant check below is what
        // makes this safe, and it fails unless a person approved this exact path.
        let promoted = policy.promote_confined_read("file_write", "path", path)?;
        policy.before_granted_action("file_write", "path", &promoted)?;
        policy.before_action("file_write", "contents", Role::Content, contents)?;

        let relative = promoted
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the path was not trusted after endorsement",
            })?;

        let resolved = self.resolve(&relative)?;
        self.record_backup(&resolved);

        let proof = policy.authorise_content_release("file_write", "contents");
        let body = contents.clone().declassify(&proof);

        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WorkspaceError::Io {
                path: relative.clone(),
                detail: e.to_string(),
            })?;
        }

        std::fs::write(&resolved, body).map_err(|e| WorkspaceError::Io {
            path: relative,
            detail: e.to_string(),
        })?;

        Ok(resolved)
    }

    /// Write a file. The path is routing; the contents are content.
    ///
    /// Untrusted contents are permitted, and that asymmetry is the point. What is refused
    /// is an untrusted *path*, or contents that are still private.
    pub fn write<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        path: &Labelled<String>,
        contents: &Labelled<String>,
    ) -> Result<PathBuf, WorkspaceError> {
        policy.before_capability(Capability::FileWrite)?;
        policy.before_action("file_write", "path", Role::Routing, path)?;
        policy.before_action("file_write", "contents", Role::Content, contents)?;

        let relative = path
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the path was not trusted",
            })?;

        let resolved = self.resolve(&relative)?;

        // Both gates have passed, so the bytes may be released to the write.
        let proof = policy.authorise_content_release("file_write", "contents");
        let body = contents.clone().declassify(&proof);

        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WorkspaceError::Io {
                path: relative.clone(),
                detail: e.to_string(),
            })?;
        }

        self.record_backup(&resolved);

        std::fs::write(&resolved, body).map_err(|e| WorkspaceError::Io {
            path: relative,
            detail: e.to_string(),
        })?;

        Ok(resolved)
    }

    /// Keep what a path holds before this turn overwrites it.
    ///
    /// The first write of a turn is the one worth keeping: a path written twice was already
    /// changed by the first, so the second write's contents are this turn's doing and rewinding
    /// to them would leave the turn half undone.
    fn record_backup(&self, resolved: &Path) {
        let Ok(mut backups) = self.backups.lock() else {
            return;
        };
        if backups.iter().any(|backup| backup.path == resolved) {
            return;
        }
        let was = std::fs::read(resolved).ok();
        backups.push(Backup {
            path: resolved.to_path_buf(),
            was,
        });
    }

    /// What this turn has written so far, clearing it so the next turn starts with none.
    pub fn take_backups(&self) -> Vec<Backup> {
        let Ok(mut guard) = self.backups.lock() else {
            return Vec::new();
        };
        std::mem::take(&mut *guard)
    }

    /// Put back what a turn wrote over.
    ///
    /// A path whose file did not exist is removed again.
    pub fn restore_backups(&self, backups: Vec<Backup>) {
        for backup in backups {
            match backup.was {
                Some(bytes) => {
                    let _ = std::fs::write(&backup.path, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(&backup.path);
                }
            }
        }
    }
}

/// The label a workspace read produces, exposed for callers that need to reason about
/// it without performing a read.
pub fn read_label() -> Label {
    Label::untrusted_private()
}

/// Caps on directory walks, so a large tree cannot stall a turn or flood the model's
/// context. Truncation is size hygiene, not filtering: nothing is inspected to decide
/// what to drop.
pub(crate) const MAX_ENTRIES: usize = 2_000;

/// How many files a *search* may walk before it gives up on the rest.
///
/// Far above [`MAX_ENTRIES`] because the two caps guard different things. A listing's paths
/// are the answer and every one of them is spent on context, so a listing is capped at what
/// is worth reading. A search's paths are never shown: only matching lines are, and those
/// have their own cap in [`MAX_MATCHES`]. Holding a search to a listing's budget bought no
/// context back and cost whole subtrees, which is the failure that matters: a search that
/// stops after a couple of thousand files reports nothing for a needle it never looked for,
/// and nothing reads as an answer.
///
/// Walking is cheap: a path is a stat and a string. Reading is not, which is what
/// [`MAX_SEARCH_TIME`] is for.
pub const MAX_SEARCH_FILES: usize = 100_000;

const MAX_MATCHES: usize = 200;
const MAX_MATCH_LINE: usize = 500;

/// How long a search may spend opening files.
///
/// The match cap already stops a *productive* search early. This is for the other one: a
/// pattern that matches nothing is read to the end of the tree, so on a large repository the
/// worst case is every file. A wall-clock budget bounds that without bounding the useful
/// case, and stopping is reported the same way the entry cap is, since the answer is partial
/// either way, and what the reader must not do is take it for complete.
const MAX_SEARCH_TIME: Duration = Duration::from_secs(10);

/// Caps on a single paged read.
///
/// A turn re-sends the whole message history each round, so the cost of one oversized read
/// is paid repeatedly. These bound a page rather than the file: the rest stays reachable by
/// asking for a later offset.
const MAX_PAGE_LINES: usize = 500;
const MAX_LINE: usize = 2_000;

/// Bytes inspected when deciding whether a file is text.
const SNIFF_BYTES: usize = 8_192;

/// Directories skipped when walking a tree.
///
/// Version control, build output and vendored dependencies would dominate a listing without
/// adding anything a task needs. This is size hygiene applied to *directory names*, not to
/// content: nothing is read to decide, so it cannot be steered by what a file contains.
///
/// A fixed list rather than the project's own ignore file, and deliberately. Reading
/// `.gitignore` would generalise better, being how a search tool learns each repository's
/// own idea of noise, but it would decide what to walk from the contents of a file in the
/// tree being walked, and a tree that can hide its own files from search is a tree that can
/// hide them from review. The names below are ones no project uses for its own sources, so
/// skipping them needs nobody's word for it.
///
/// Vendored code is the entry that earns its place by experience: a search for a common word
/// spent its entire budget inside a Rust crate mirror and reported documentation comments
/// about the wrong meaning of the word, having never reached the project.
/// Whether a directory of this name is one a walk steps over.
///
/// Shared so that everything walking the tree skips the same names. A pattern expanded for a
/// command line and a listing shown to a person that disagreed about `node_modules` would be two
/// different ideas of what the tree contains.
pub fn is_ignored_directory(name: &str) -> bool {
    IGNORED_DIRECTORIES.contains(&name)
}

const IGNORED_DIRECTORIES: &[&str] = &[
    // Version control.
    ".git",
    ".hg",
    ".svn",
    // Build output and caches.
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".parcel-cache",
    ".turbo",
    ".gradle",
    ".cache",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    "__pycache__",
    "coverage",
    ".nyc_output",
    ".terraform",
    ".stack-work",
    // Dependencies, fetched or vendored. `out` and `bin` are deliberately absent: plenty of
    // projects keep real sources under those names.
    "node_modules",
    "bower_components",
    "vendor",
    "third_party",
    "thirdparty",
    "Pods",
    "Carthage",
    "site-packages",
    ".venv",
    "venv",
    ".bundle",
];

/// Shorten a string to at most `limit` bytes without splitting a character.
///
/// `String::truncate` panics if the index is not a character boundary, so a matching line
/// containing multi-byte text could otherwise bring down the turn. Truncating to the
/// nearest boundary at or below the limit keeps the cap a cap.
fn truncate_on_char_boundary(text: &mut String, limit: usize) {
    if text.len() <= limit {
        return;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

/// Whether a byte run looks like binary rather than text.
///
/// A null byte is decisive, since no text file contains one. Beyond that, a high proportion of
/// control characters means the same thing without needing a file-type list to be kept up
/// to date. Only the head is inspected, since the answer does not improve by reading more.
/// Fill as much of `buffer` as the file has, since one read is not obliged to return it all.
fn read_up_to(file: &mut std::fs::File, buffer: &mut [u8]) -> std::io::Result<usize> {
    use std::io::Read;
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn looks_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    if head.is_empty() {
        return false;
    }
    if head.contains(&0) {
        return true;
    }
    // Tab, newline, carriage return and form feed are expected in text; other low bytes
    // are not.
    let control = head
        .iter()
        .filter(|b| **b < 32 && !matches!(**b, 9 | 10 | 12 | 13))
        .count();
    control * 100 / head.len() > 30
}

/// A bounded window of a file's lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The lines in this window, each capped at [`MAX_LINE`].
    pub lines: Vec<String>,
    /// Whether the file ends with a newline.
    ///
    /// Lost otherwise: the lines are joined back together with newlines between them and none
    /// after, so a file that went through a slot came back a byte shorter than it went in. That
    /// is a change to every file processed this way, and it shows up in the next diff somebody
    /// reads as "no newline at end of file".
    pub ends_with_newline: bool,
    /// 1-based number of the first line returned.
    pub first_line: usize,
    /// Lines in the whole file, so a caller can tell there is more to ask for.
    pub total_lines: usize,
    /// How many returned lines were individually shortened.
    pub long_lines: usize,
}

impl Page {
    /// 1-based line number just past this window, when the file continues.
    pub fn next_line(&self) -> Option<usize> {
        let past = self.first_line + self.lines.len();
        (past <= self.total_lines).then_some(past)
    }
}

/// One grep hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Workspace-relative path.
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    /// The matching line, truncated to [`MAX_MATCH_LINE`].
    pub text: String,
}

/// The result of a directory listing.
///
/// Carries whether a cap was reached, because a model shown exactly [`MAX_ENTRIES`] paths
/// with no notice will reason as though it saw the whole tree. Silent truncation is worse
/// than a short answer: it looks like completeness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub files: Vec<String>,
    /// Directories the walk stopped at because it had reached the depth it was given.
    ///
    /// Empty when the walk was unbounded, where every directory is descended into and the files
    /// beneath it are the listing. A bounded walk reports them because a listing of names with
    /// nothing said about the directories beside them describes a tree with no branches, and a
    /// planner reading one concludes the project has no source directory.
    pub directories: Vec<String>,
    /// Whether files were left out because a cap was reached.
    pub truncated: bool,
}

/// The result of a content search. Reports truncation for the same reason as [`Listing`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matches {
    pub matches: Vec<Match>,
    /// Whether matches were left out because the match cap was reached.
    pub truncated: bool,
    /// Whether files were left unopened because the walk hit its entry cap.
    ///
    /// A separate fact from `truncated` and the more dangerous of the two: a search that stopped
    /// short of the tree reports no matches for a needle it never looked for, which reads exactly
    /// like the needle not being there.
    pub unvisited: bool,
    /// Whether reading stopped because [`MAX_SEARCH_TIME`] ran out.
    ///
    /// Dangerous in the same way as `unvisited`, and for the same reason: what was not read
    /// cannot have matched.
    pub timed_out: bool,
    /// How many files the `include` glob selected.
    ///
    /// The number that separates the two ways a search comes back empty. Zero means the glob
    /// picked no files at all, so nothing was ever read and the result says nothing about
    /// whether the needle is in the tree: a broken query, not evidence. Anything above zero
    /// means files were read and the needle was not in them, which is evidence. Rendered as
    /// one message, those two are indistinguishable, and a reader who cannot tell them apart
    /// treats a typo as proof of absence.
    pub considered: usize,
    /// How many of the selected files were actually opened.
    ///
    /// Below `considered` when a cap or the clock stopped the read early. Reported so the
    /// question "was this search complete?" has an answer in the result rather than in
    /// another call.
    pub searched: usize,
}

impl Workspace {
    /// List files under a workspace-relative directory.
    ///
    /// The directory is routing, so content cannot choose where to look. The resulting
    /// *paths* are untrusted-private: a filename is content the user's tree supplied,
    /// and a file could be named to look like an instruction.
    /// `pattern` narrows the result to matching paths. It is routing like the directory: a
    /// filter chooses what is looked at, so untrusted text must not supply one.
    pub fn list<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        directory: &Labelled<String>,
        pattern: Option<&Labelled<String>>,
        depth: Option<usize>,
    ) -> Result<Labelled<Listing>, WorkspaceError> {
        policy.before_capability(Capability::FileRead)?;
        policy.before_action("file_list", "directory", Role::Routing, directory)?;

        let relative = directory
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the directory was not trusted",
            })?;

        let glob = match pattern {
            Some(pattern) => {
                policy.before_action("file_list", "pattern", Role::Routing, pattern)?;
                Some(
                    pattern
                        .clone()
                        .into_trusted()
                        .map_err(|_| WorkspaceError::Invalid {
                            path: "<untrusted>".into(),
                            reason: "the pattern was not trusted",
                        })?,
                )
            }
            None => None,
        };

        let root = self.resolve(&relative)?;

        let mut found = Vec::new();
        let mut stopped_at = Vec::new();
        // Ignored here: what a listing left out is the entry it drops below, which the count
        // answers exactly.
        let patterns = glob.as_deref().map(crate::glob::expand);
        let _ = self.walk_filtered(
            &root,
            patterns.as_deref(),
            depth,
            MAX_ENTRIES,
            &mut found,
            &mut stopped_at,
        )?;
        found.sort();
        stopped_at.sort();

        // Labelled after the walk, because which paths were visited is not known before it. A
        // listing is trusted only if every path in it is. A directory name is a name out of the
        // same tree, so it is observed with the files rather than beside them.
        let label = policy.observe_paths(
            Capability::FileRead,
            found.iter().chain(stopped_at.iter()).map(String::as_str),
        )?;

        // `walk` collects one entry past the cap so reaching it is detectable. Which entries
        // survive is down to traversal order, so a truncated listing is a sample of the tree
        // rather than its alphabetical head, hence saying so matters. The order is at least
        // the same order every time, which is the walk's doing rather than this sort's: what
        // is sorted here is what a walk kept, and sorting after a cap cannot choose what it
        // kept.
        let truncated = found.len() + stopped_at.len() > MAX_ENTRIES;
        found.truncate(MAX_ENTRIES);
        stopped_at.truncate(MAX_ENTRIES.saturating_sub(found.len()));

        Ok(Labelled::new(
            Listing {
                files: found,
                directories: stopped_at,
                truncated,
            },
            label,
        ))
    }

    /// Search file contents for a literal substring.
    ///
    /// The pattern and directory are routing; the matches are untrusted-private, exactly
    /// like a file read. Matching is a plain substring test rather than a regex: a
    /// pattern is cheap to get wrong, and a catastrophically backtracking regex supplied
    /// through a turn would be a denial-of-service vector.
    /// Find lines containing any of `patterns` in files beneath `directory`.
    ///
    /// More than one pattern because the alternative is more than one call. A search is a
    /// round trip, and a round trip is the expensive part of a turn: the tool itself returns
    /// in milliseconds while the model it answers takes seconds to ask again. Looking for
    /// three spellings of the same identifier is one question, and it should cost one answer.
    ///
    /// Alternation rather than a regular expression, so this stays what it says it is: every
    /// pattern is a literal, matched with `contains`, and a line matching any of them matches.
    /// That keeps the property a regex engine would give up, work proportional to the input
    /// with no pattern able to make the search expensive, while covering the case that
    /// actually sends a model round the loop again.
    pub fn grep<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        patterns: &[Labelled<String>],
        directory: &Labelled<String>,
        include: Option<&Labelled<String>>,
        case_sensitive: bool,
    ) -> Result<Labelled<Matches>, WorkspaceError> {
        policy.before_capability(Capability::FileRead)?;
        for pattern in patterns {
            policy.before_action("file_grep", "pattern", Role::Routing, pattern)?;
        }
        policy.before_action("file_grep", "directory", Role::Routing, directory)?;

        let relative = directory
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the directory was not trusted",
            })?;

        if patterns.is_empty() {
            return Err(WorkspaceError::Invalid {
                path: relative,
                reason: "no search pattern was given",
            });
        }

        let mut needles = Vec::with_capacity(patterns.len());
        for pattern in patterns {
            let needle = pattern
                .clone()
                .into_trusted()
                .map_err(|_| WorkspaceError::Invalid {
                    path: "<untrusted>".into(),
                    reason: "the pattern was not trusted",
                })?;
            if needle.is_empty() {
                return Err(WorkspaceError::Invalid {
                    path: relative,
                    reason: "the search pattern was empty",
                });
            }
            // Folded once here rather than per line. `to_lowercase` is Unicode-aware and
            // allocates, so doing it inside the match loop would put a per-line allocation on
            // every file in the tree.
            needles.push(if case_sensitive {
                needle
            } else {
                needle.to_lowercase()
            });
        }

        // Which files are searched is routing, exactly like the directory.
        let glob = match include {
            Some(include) => {
                policy.before_action("file_grep", "include", Role::Routing, include)?;
                Some(
                    include
                        .clone()
                        .into_trusted()
                        .map_err(|_| WorkspaceError::Invalid {
                            path: "<untrusted>".into(),
                            reason: "the include pattern was not trusted",
                        })?,
                )
            }
            None => None,
        };

        let root = self.resolve(&relative)?;

        let mut paths = Vec::new();
        // Whether every file was reached, which the count cannot answer: a tree of exactly the
        // cap fills `paths` without a single file being left out.
        let mut ignored = Vec::new();
        // Expanded once for the whole walk, not once per path.
        let expanded = glob.as_deref().map(crate::glob::expand);
        let unvisited = self.walk_filtered(
            &root,
            expanded.as_deref(),
            None,
            self.search_files,
            &mut paths,
            &mut ignored,
        )?;
        paths.sort();
        let considered = paths.len();

        // Trusted only if every file the search reads is trusted.
        let label = policy.observe_paths(Capability::FileRead, paths.iter().map(String::as_str))?;

        // Collected one past the cap for the same reason as `walk`: reaching the limit has
        // to be distinguishable from happening to have exactly that many matches.
        let mut matches = Vec::new();
        let mut searched = 0usize;
        let mut timed_out = false;
        let started = Instant::now();
        for path in paths {
            if matches.len() > MAX_MATCHES {
                break;
            }
            // Checked per file rather than per line: the clock is here to bound a walk over a
            // large tree, and a single file cannot be large enough to matter beside that.
            if started.elapsed() >= MAX_SEARCH_TIME {
                timed_out = true;
                break;
            }
            let absolute = self.root.join(&path);
            // Unreadable or non-UTF8 files are skipped rather than failing the search:
            // a binary in the tree should not make grep unusable.
            let Ok(contents) = std::fs::read_to_string(&absolute) else {
                continue;
            };
            searched += 1;
            for (index, line) in contents.lines().enumerate() {
                if matches.len() > MAX_MATCHES {
                    break;
                }
                let hit = if case_sensitive {
                    needles.iter().any(|needle| line.contains(needle))
                } else {
                    // One allocation per line, and only on the case-insensitive path, which
                    // is the one that asked for it.
                    let folded = line.to_lowercase();
                    needles.iter().any(|needle| folded.contains(needle))
                };
                if hit {
                    let mut text = line.to_string();
                    truncate_on_char_boundary(&mut text, MAX_MATCH_LINE);
                    matches.push(Match {
                        path: path.clone(),
                        line: index + 1,
                        text,
                    });
                }
            }
        }

        let truncated = matches.len() > MAX_MATCHES;
        matches.truncate(MAX_MATCHES);

        Ok(Labelled::new(
            Matches {
                matches,
                truncated,
                unvisited,
                timed_out,
                considered,
                searched,
            },
            label,
        ))
    }

    /// Collect workspace-relative paths of regular files beneath `directory`.
    ///
    /// Symlinks are not followed, which is the same escape `resolve` rejects for a named path.
    ///
    /// Stops once one entry *past* `limit` is collected. The extra entry is what lets the
    /// caller distinguish a tree that exactly fills the cap from one that overflows it, so
    /// truncation can be reported rather than guessed at.
    ///
    /// `patterns`, when given, keeps only paths matching at least one of them. They arrive
    /// already expanded by [`crate::glob::expand`], because one walk applies the same pattern
    /// to every path it sees and expanding per path would allocate once per file. The filter
    /// is applied before the cap, so the cap bounds *matches* rather than files examined.
    /// Filtering afterwards would make a narrow pattern return nothing in a large tree, which
    /// looks identical to the file being absent.
    ///
    /// Answers whether it stopped at the cap with entries still unvisited, which the length of
    /// `out` cannot: a directory holding exactly one past the cap fills it without anything
    /// being left behind.
    ///
    /// `remaining`, when given, is how many more levels may be descended. A directory at the
    /// boundary is put in `stopped_at` instead of being walked, so the caller can say the tree
    /// continues there. The filter does not apply to those: `patterns` narrows which files are
    /// reported, and the shape of the tree is not a file.
    ///
    /// Entries are sorted within each directory, and a directory's own files are taken before
    /// any of its subdirectories are descended into. Neither is cosmetic. `read_dir` order is
    /// the filesystem's, so a walk that stopped at a cap used to keep an arbitrary subset and
    /// the same search could answer differently on two machines. Sorting makes the sample
    /// reproducible, and taking a directory's files first means every directory the walk
    /// reaches contributes its own contents before the walk disappears into the first subtree
    /// under it. A partial answer cannot be helped once the cap is reached. Which part it
    /// is can.
    fn walk_filtered(
        &self,
        directory: &Path,
        patterns: Option<&[String]>,
        remaining: Option<usize>,
        limit: usize,
        out: &mut Vec<String>,
        stopped_at: &mut Vec<String>,
    ) -> Result<bool, WorkspaceError> {
        let entries = std::fs::read_dir(directory).map_err(|e| WorkspaceError::Io {
            path: self.relative_display(directory),
            detail: e.to_string(),
        })?;

        // Collected before anything is reported so the two passes below can be ordered
        // independently of how the filesystem happened to hand them over.
        let mut files = Vec::new();
        let mut directories = Vec::new();
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            // A link pointing outside the workspace would otherwise pull external files
            // into a listing.
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                // Version control, build output and vendored dependencies would dominate a
                // listing without adding anything a task needs.
                let name = entry.file_name();
                if is_ignored_directory(name.to_string_lossy().as_ref()) {
                    continue;
                }
                directories.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
        files.sort();
        directories.sort();

        for path in files {
            if out.len() + stopped_at.len() > limit {
                return Ok(true);
            }
            let relative = self.relative_display(&path);
            match patterns {
                Some(patterns) if !crate::glob::matches_any(patterns, &relative) => continue,
                _ => out.push(relative),
            }
        }

        for path in directories {
            if out.len() + stopped_at.len() > limit {
                return Ok(true);
            }
            if remaining.is_some_and(|left| left <= 1) {
                stopped_at.push(self.relative_display(&path));
                continue;
            }
            // Propagated rather than left to the next iteration's check, which a directory
            // with nothing after it never reaches.
            if self.walk_filtered(
                &path,
                patterns,
                remaining.map(|left| left - 1),
                limit,
                out,
                stopped_at,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// How a path is named back to the caller.
    ///
    /// Relative to the primary root for a file in the project, and absolute for one in an added
    /// directory. That is the same spelling each would have to be given to reach the file again, so
    /// a listing can be read and acted on without knowing which tree an entry came from.
    fn relative_display(&self, path: &Path) -> String {
        match path.strip_prefix(&self.root) {
            Ok(relative) => relative.to_string_lossy().to_string(),
            Err(_) => path.to_string_lossy().to_string(),
        }
    }
}
