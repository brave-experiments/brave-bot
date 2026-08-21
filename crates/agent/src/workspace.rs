//! Label-aware filesystem tools.
//!
//! Both operations split into a **routing** part and a **content** part, and the split
//! is what the policy gate checks:
//!
//! - `read(path)` — the path is routing. It must be `(T,pub)`, so untrusted content can
//!   never choose which file is read.
//! - `write(path, contents)` — the path is routing, the contents are content. Untrusted
//!   text may be written *into* a file it could not choose.
//!
//! Paths are also confined to a workspace root. That is a second, independent check:
//! the routing label stops content from *supplying* a path, while confinement stops a
//! trusted-but-wrong path from escaping the project.

use bua_core::capability::Capability;
use bua_core::event::{Role, Sink};
use bua_core::label::Label;
use bua_core::policy::{Denial, Policy};
use bua_core::value::Labelled;
use std::fmt;
use std::path::{Component, Path, PathBuf};

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
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl From<Denial> for WorkspaceError {
    fn from(value: Denial) -> Self {
        Self::Denied(value)
    }
}

/// A directory that file operations are confined to.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Create a workspace at `root`, which must exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, WorkspaceError> {
        let root = root.into();
        let canonical = root.canonicalize().map_err(|e| WorkspaceError::Io {
            path: root.display().to_string(),
            detail: e.to_string(),
        })?;
        Ok(Self { root: canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a relative path inside the workspace.
    ///
    /// Rejects absolute paths and any `..` component. `..` is rejected before touching
    /// the filesystem rather than by canonicalising afterwards, because the target of a
    /// write may not exist yet, and a check that only works for existing files would
    /// leave writes unprotected.
    fn resolve(&self, relative: &str) -> Result<PathBuf, WorkspaceError> {
        let candidate = Path::new(relative);

        if candidate.is_absolute() {
            return Err(WorkspaceError::Invalid {
                path: relative.to_string(),
                reason: "must be relative to the workspace root",
            });
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
        // the root — this is what catches a symlink pointing out of the workspace.
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

    /// Read a file. The path is checked as routing, so it must be `(T,pub)`.
    ///
    /// The contents come back labelled untrusted-private: a workspace file may contain
    /// anything, including text fetched from the network earlier, and it is the user's
    /// data.
    pub fn read<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        path: &Labelled<String>,
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

        let resolved = self.resolve(&relative)?;
        let label = policy.observe(Capability::FileRead)?;

        let contents = std::fs::read_to_string(&resolved).map_err(|e| WorkspaceError::Io {
            path: relative,
            detail: e.to_string(),
        })?;

        Ok(Labelled::new(contents, label))
    }

    /// Write a file. The path is routing; the contents are content.
    ///
    /// Untrusted contents are permitted — that asymmetry is the point. What is refused
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

        std::fs::write(&resolved, body).map_err(|e| WorkspaceError::Io {
            path: relative,
            detail: e.to_string(),
        })?;

        Ok(resolved)
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
const MAX_ENTRIES: usize = 2_000;
const MAX_MATCHES: usize = 200;
const MAX_MATCH_LINE: usize = 500;

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

impl Workspace {
    /// List files under a workspace-relative directory.
    ///
    /// The directory is routing, so content cannot choose where to look. The resulting
    /// *paths* are untrusted-private: a filename is content the user's tree supplied,
    /// and a file could be named to look like an instruction.
    pub fn list<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        directory: &Labelled<String>,
    ) -> Result<Labelled<Vec<String>>, WorkspaceError> {
        policy.before_capability(Capability::FileRead)?;
        policy.before_action("file_list", "directory", Role::Routing, directory)?;

        let relative = directory
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the directory was not trusted",
            })?;

        let root = self.resolve(&relative)?;
        let label = policy.observe(Capability::FileRead)?;

        let mut found = Vec::new();
        self.walk(&root, &mut found)?;
        found.sort();
        found.truncate(MAX_ENTRIES);

        Ok(Labelled::new(found, label))
    }

    /// Search file contents for a literal substring.
    ///
    /// The pattern and directory are routing; the matches are untrusted-private, exactly
    /// like a file read. Matching is a plain substring test rather than a regex: a
    /// pattern is cheap to get wrong, and a catastrophically backtracking regex supplied
    /// through a turn would be a denial-of-service vector.
    pub fn grep<S: Sink>(
        &self,
        policy: &mut Policy<'_, S>,
        pattern: &Labelled<String>,
        directory: &Labelled<String>,
    ) -> Result<Labelled<Vec<Match>>, WorkspaceError> {
        policy.before_capability(Capability::FileRead)?;
        policy.before_action("file_grep", "pattern", Role::Routing, pattern)?;
        policy.before_action("file_grep", "directory", Role::Routing, directory)?;

        let needle = pattern
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the pattern was not trusted",
            })?;
        let relative = directory
            .clone()
            .into_trusted()
            .map_err(|_| WorkspaceError::Invalid {
                path: "<untrusted>".into(),
                reason: "the directory was not trusted",
            })?;

        if needle.is_empty() {
            return Err(WorkspaceError::Invalid {
                path: relative,
                reason: "the search pattern was empty",
            });
        }

        let root = self.resolve(&relative)?;
        let label = policy.observe(Capability::FileRead)?;

        let mut paths = Vec::new();
        self.walk(&root, &mut paths)?;
        paths.sort();

        let mut matches = Vec::new();
        for path in paths {
            if matches.len() >= MAX_MATCHES {
                break;
            }
            let absolute = self.root.join(&path);
            // Unreadable or non-UTF8 files are skipped rather than failing the search:
            // a binary in the tree should not make grep unusable.
            let Ok(contents) = std::fs::read_to_string(&absolute) else {
                continue;
            };
            for (index, line) in contents.lines().enumerate() {
                if matches.len() >= MAX_MATCHES {
                    break;
                }
                if line.contains(&needle) {
                    let mut text = line.to_string();
                    text.truncate(MAX_MATCH_LINE);
                    matches.push(Match {
                        path: path.clone(),
                        line: index + 1,
                        text,
                    });
                }
            }
        }

        Ok(Labelled::new(matches, label))
    }

    /// Collect workspace-relative paths of regular files beneath `directory`.
    ///
    /// Symlinks are not followed: a link pointing outside the workspace would otherwise
    /// pull external files into a listing, which is the same escape `resolve` rejects
    /// for a named path.
    fn walk(&self, directory: &Path, out: &mut Vec<String>) -> Result<(), WorkspaceError> {
        let entries = std::fs::read_dir(directory).map_err(|e| WorkspaceError::Io {
            path: self.relative_display(directory),
            detail: e.to_string(),
        })?;

        for entry in entries.flatten() {
            if out.len() >= MAX_ENTRIES {
                return Ok(());
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };

            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                // Version control and build output would dominate a listing without
                // adding anything a task needs.
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name == ".git" || name == "target" || name == "node_modules" {
                    continue;
                }
                self.walk(&path, out)?;
                continue;
            }
            if kind.is_file() {
                out.push(self.relative_display(&path));
            }
        }
        Ok(())
    }

    fn relative_display(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    }
}
