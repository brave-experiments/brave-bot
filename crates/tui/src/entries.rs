//! Workspace entries offered while a file reference is being typed.
//!
//! # What an `@` reference means
//!
//! Naming a file with `@` puts its **contents into the turn as trusted input**, exactly as
//! `--file` does on the command line, because the user named it and the user is the one party whose
//! word makes something trusted. That is the whole point: a file the planner may read and compare
//! and act on, rather than a reference it can only carry.
//!
//! So this list exists to make that choice an informed one. It is drawn from the directory itself
//! rather than from anything a model said, it is shown to the person typing, and the file it names
//! becomes context only once they press Enter on the line. The keystroke is the grant, the same way
//! it is for a prompt recalled out of history.
//!
//! Nothing here is a decision derived from untrusted content. Filenames are content, and this
//! walks the directory to show them to a person, which is the release
//! [`bravebot_core::policy::Policy::names_for_display`] already makes for the same reason: the user owns
//! the workspace, and an interface that will not tell them which files are in it has protected
//! them from nothing. No name reaches a model from here.

use std::path::Path;

/// How many entries are offered at once.
///
/// A directory of ten thousand files would otherwise be a list nobody can read and a redraw for
/// every keystroke. Narrowing is what finds a file; the cap only bounds the first look.
const MAX_ENTRIES: usize = 40;

/// One thing in the workspace that a reference could name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Workspace-relative, with a trailing slash for a directory so it reads as one.
    pub path: String,
    /// Whether it is a directory, which completes to a path that can be typed further.
    pub is_directory: bool,
}

/// Entries matching a half-typed reference, directories first and then files, each alphabetical.
///
/// `typed` is what follows the `@`. An empty one lists the workspace root. Anything with a slash in
/// it lists the directory named up to the last slash, so `crates/t` offers what is in `crates`.
///
/// Directories come first because a reference is usually typed by walking into one, and being able
/// to go deeper matters more than the file that happens to sort first.
pub fn matching(root: &Path, typed: &str) -> Vec<Entry> {
    // Refused rather than resolved: `..` would walk out of the workspace, and an absolute path
    // names something the reference syntax has no business reaching.
    if typed.contains("..") || typed.starts_with('/') {
        return Vec::new();
    }

    let (directory, prefix) = match typed.rsplit_once('/') {
        Some((directory, prefix)) => (directory, prefix),
        None => ("", typed),
    };

    let listed = root.join(directory);
    // Confined the same way every other path is: a symlinked subdirectory pointing out of the
    // workspace must not become a way to browse the filesystem.
    let Ok(canonical) = listed.canonicalize() else {
        return Vec::new();
    };
    let Ok(canonical_root) = root.canonicalize() else {
        return Vec::new();
    };
    if !canonical.starts_with(&canonical_root) {
        return Vec::new();
    }

    let Ok(reading) = std::fs::read_dir(&canonical) else {
        return Vec::new();
    };

    let mut entries: Vec<Entry> = reading
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(prefix) {
                return None;
            }
            // Skipped rather than offered: a reference into `.git` is never what was meant, and
            // offering the whole of it buries everything else.
            if is_noise(&name) {
                return None;
            }
            let is_directory = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let path = if directory.is_empty() {
                name
            } else {
                format!("{directory}/{name}")
            };
            Some(Entry {
                path: if is_directory {
                    format!("{path}/")
                } else {
                    path
                },
                is_directory,
            })
        })
        .collect();

    // Sorted so the list is the same on every platform: `read_dir` returns whatever order the
    // filesystem holds, which would otherwise reshuffle the offered entries between machines.
    entries.sort_by(|a, b| {
        b.is_directory
            .cmp(&a.is_directory)
            .then_with(|| a.path.cmp(&b.path))
    });
    entries.truncate(MAX_ENTRIES);
    entries
}

/// Directories nobody means to reference, and which would bury everything else.
fn is_noise(name: &str) -> bool {
    matches!(name, ".git" | "node_modules" | "target")
}

/// What follows an `@` at the end of the line, if the line is being typed towards a reference.
///
/// `None` unless the last word begins with `@`, so a reference already finished by a space is left
/// alone and an ordinary prompt offers nothing. That is what closes the list.
pub fn typed_reference(line: &str) -> Option<&str> {
    let last = line.split_whitespace().next_back()?;
    // Only while it is still being typed: a space after a reference means the user moved on.
    if line.ends_with(char::is_whitespace) {
        return None;
    }
    last.strip_prefix('@')
}

/// Every file named with `@` in a line, in the order they were written.
///
/// This is what becomes a turn's context. A trailing slash is dropped, since a directory is a place
/// to type through rather than a file to read, and one named anyway is not a file to include.
pub fn referenced(line: &str) -> Vec<String> {
    line.split_whitespace()
        .filter_map(|word| word.strip_prefix('@'))
        .filter(|path| !path.is_empty() && !path.ends_with('/'))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch workspace, removed with the test.
    struct Scratch {
        path: std::path::PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("bravebot-entries-{name}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("crates/tui")).expect("create");
            std::fs::create_dir_all(path.join("target")).expect("create");
            std::fs::create_dir_all(path.join(".git")).expect("create");
            std::fs::create_dir_all(path.join("node_modules")).expect("create");
            std::fs::write(path.join("Cargo.toml"), "").expect("write");
            std::fs::write(path.join("Makefile"), "").expect("write");
            std::fs::write(path.join("crates/tui/lib.rs"), "").expect("write");
            Self { path }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn paths(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.path.as_str()).collect()
    }

    /// An empty reference lists the root, directories first, so walking deeper is the first thing
    /// offered.
    #[test]
    fn an_empty_reference_lists_the_root_with_directories_first() {
        let scratch = Scratch::new("root");
        let offered = matching(&scratch.path, "");
        assert_eq!(paths(&offered), vec!["crates/", "Cargo.toml", "Makefile"]);
    }

    /// A prefix narrows the list, which is what makes typing a path possible at all.
    #[test]
    fn a_prefix_narrows_the_list() {
        let scratch = Scratch::new("prefix");
        assert_eq!(paths(&matching(&scratch.path, "Ma")), vec!["Makefile"]);
        assert!(matching(&scratch.path, "zz").is_empty());
    }

    /// A slash lists the directory it names, so a reference is typed by walking into one.
    #[test]
    fn a_slash_lists_what_is_inside_that_directory() {
        let scratch = Scratch::new("nested");
        // Whole paths, not just the last segment: what is offered is what gets typed.
        assert_eq!(
            paths(&matching(&scratch.path, "crates/")),
            vec!["crates/tui/"]
        );
        assert_eq!(
            paths(&matching(&scratch.path, "crates/tui/l")),
            vec!["crates/tui/lib.rs"]
        );
    }

    /// Build output and history are never what a reference means, and offering them buries the rest.
    #[test]
    fn noise_directories_are_not_offered() {
        let scratch = Scratch::new("noise");
        let offered = matching(&scratch.path, "");
        let root = paths(&offered);
        assert!(!root.contains(&"target/"), "build output was offered");
        assert!(!root.contains(&".git/"), "version control was offered");
        assert!(
            !root.contains(&"node_modules/"),
            "dependencies were offered"
        );
    }

    /// The completion must not become a way to browse the filesystem: `..` and an absolute path are
    /// refused rather than resolved, exactly as the workspace refuses them.
    #[test]
    fn a_reference_cannot_climb_out_of_the_workspace() {
        let scratch = Scratch::new("escape");
        assert!(matching(&scratch.path, "../").is_empty());
        assert!(matching(&scratch.path, "../../etc/").is_empty());
        assert!(matching(&scratch.path, "/etc/").is_empty());
    }

    /// A reference is offered while the word is being typed, and left alone once a space says the
    /// user moved on.
    #[test]
    fn what_counts_as_a_reference_being_typed() {
        assert_eq!(typed_reference("look at @Car"), Some("Car"));
        assert_eq!(typed_reference("@"), Some(""));
        assert_eq!(typed_reference("@Cargo.toml "), None, "finished by a space");
        assert_eq!(typed_reference("an ordinary prompt"), None);
        assert_eq!(typed_reference(""), None);
        // An address is not a reference: only the last word counts, and this one is not it.
        assert_eq!(typed_reference("mail me@example.com now"), None);
    }

    /// Every referenced file becomes context, since each is one the user named.
    #[test]
    fn every_referenced_file_is_collected() {
        assert_eq!(
            referenced("compare @a.rs with @b.rs please"),
            vec!["a.rs".to_string(), "b.rs".to_string()]
        );
        assert!(referenced("no references here").is_empty());
    }

    /// A directory is a place to type through, not a file to read, so it is not collected.
    #[test]
    fn a_directory_is_not_collected_as_a_file() {
        assert!(referenced("look in @crates/").is_empty());
        assert_eq!(referenced("@crates/tui/lib.rs"), vec!["crates/tui/lib.rs"]);
    }

    /// A bare `@` names nothing, so it is a character in a sentence rather than a file.
    #[test]
    fn a_bare_at_sign_names_nothing() {
        assert!(referenced("what does @ do").is_empty());
    }
}
