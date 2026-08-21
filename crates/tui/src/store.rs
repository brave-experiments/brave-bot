//! Where global state lives on disk.
//!
//! `~/.bua` holds anything that should outlive a session. Only prompt history so far, but the
//! directory is the place for it rather than a per-project file: a question worth asking again is
//! usually worth asking in another checkout too.
//!
//! Every operation here degrades to doing nothing. A missing home directory, a read-only disk, a
//! corrupt file: none of that is worth refusing to start over, because history is a convenience
//! and the session works without it.
//!
//! # What comes back is not trusted
//!
//! A history file can be edited, and on a shared machine it can be edited by someone else. So a
//! recalled prompt is not fed to a turn: it is placed in the input box, where the user reads it
//! and presses Enter. That keystroke is what makes it trusted, exactly as typing it would have.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The directory holding global state.
const DIRECTORY: &str = ".bua";

/// The history file inside it.
const HISTORY_FILE: &str = "history";

/// Prompts kept on disk.
///
/// Bounded so the file cannot grow without limit on a machine that is used for years. The oldest
/// entries are dropped, since recall walks backwards from the newest.
const MAX_ENTRIES: usize = 1_000;

/// The longest prompt worth storing.
///
/// A pasted file would otherwise sit in history forever, and recalling it is not useful: it is far
/// past what the input box can show at once.
const MAX_ENTRY_BYTES: usize = 4_096;

/// The global state directory, or `None` when there is no home to put it in.
pub fn directory() -> Option<PathBuf> {
    // Read directly rather than taking a dependency for one variable. Absent in some daemon and
    // container environments, which is a case that has to be handled anyway.
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(Path::new(&home).join(DIRECTORY))
}

/// Read stored prompts, oldest first.
///
/// Returns an empty list when there is nothing to read, or when what is there cannot be parsed.
/// Lines are trimmed of the newline they are stored with; blank lines are skipped so a
/// hand-edited file cannot inject empty entries.
pub fn load_history() -> Vec<String> {
    let Some(path) = directory().map(|dir| dir.join(HISTORY_FILE)) else {
        return Vec::new();
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_history(&contents)
}

/// Turn a file's contents into entries.
///
/// Separate from the I/O so the decoding is testable, and because the rules matter: a prompt
/// containing a newline was stored escaped, and an over-long line is dropped rather than truncated
/// to half a question.
pub fn parse_history(contents: &str) -> Vec<String> {
    let mut entries: Vec<String> = contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| line.len() <= MAX_ENTRY_BYTES)
        .map(unescape)
        .collect();

    // A file longer than the cap is read from its end: those are the entries recall reaches first.
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    entries
}

/// Append one prompt to the history file.
///
/// Best-effort by design: a failure here must not interrupt a session, so nothing is reported.
/// Appending rather than rewriting keeps the cost independent of how much history exists.
pub fn append_history(prompt: &str) {
    if prompt.trim().is_empty() || prompt.len() > MAX_ENTRY_BYTES {
        return;
    }
    let Some(dir) = directory() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let line = format!("{}\n", escape(prompt));
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(HISTORY_FILE))
        .and_then(|mut file| file.write_all(line.as_bytes()));
}

/// Rewrite the file with exactly these entries.
///
/// Used to drop a cancelled prompt and to enforce the cap. Written to a temporary file and
/// renamed so an interrupted write cannot leave a half-truncated history.
pub fn save_history(entries: &[String]) {
    let Some(dir) = directory() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let start = entries.len().saturating_sub(MAX_ENTRIES);
    let body: String = entries[start..]
        .iter()
        .filter(|entry| !entry.trim().is_empty() && entry.len() <= MAX_ENTRY_BYTES)
        .map(|entry| format!("{}\n", escape(entry)))
        .collect();

    let temporary = dir.join("history.tmp");
    if std::fs::write(&temporary, body).is_ok() {
        let _ = std::fs::rename(&temporary, dir.join(HISTORY_FILE));
    }
}

/// Encode a prompt as one line.
///
/// A prompt may contain newlines, which would otherwise become several entries on the way back
/// in. Backslash is escaped first so the encoding round-trips.
fn escape(prompt: &str) -> String {
    prompt.replace('\\', "\\\\").replace('\n', "\\n")
}

/// Decode one stored line.
///
/// A trailing lone backslash and an unknown escape are both passed through rather than dropped: a
/// hand-edited file should come back as close to what it says as possible.
fn unescape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_is_under_the_home() {
        // Whatever HOME is in the test environment, the directory sits inside it.
        if let Some(dir) = directory() {
            assert!(dir.ends_with(DIRECTORY));
        }
    }

    #[test]
    fn entries_round_trip_through_the_file_format() {
        let entries = vec!["first".to_string(), "second".to_string()];
        let encoded: String = entries.iter().map(|e| format!("{}\n", escape(e))).collect();
        assert_eq!(parse_history(&encoded), entries);
    }

    /// A prompt with a newline must come back as one entry, not two.
    #[test]
    fn a_multiline_prompt_stays_one_entry() {
        let prompt = "explain this:\nfn main() {}";
        let encoded = format!("{}\n", escape(prompt));
        assert_eq!(parse_history(&encoded), vec![prompt.to_string()]);
    }

    /// The escape itself must survive, or a prompt about backslashes would corrupt on reload.
    #[test]
    fn backslashes_round_trip() {
        for prompt in [r"a\b", r"trailing\", r"\n literal", r"\\"] {
            let encoded = format!("{}\n", escape(prompt));
            assert_eq!(
                parse_history(&encoded),
                vec![prompt.to_string()],
                "{prompt:?} did not round-trip"
            );
        }
    }

    /// A hand-edited file must not produce empty entries, which would look like broken recall.
    #[test]
    fn blank_lines_are_skipped() {
        assert_eq!(
            parse_history("one\n\n   \ntwo\n"),
            vec!["one".to_string(), "two".to_string()]
        );
    }

    /// An unreadable or absent file is not an error: the session works without history.
    #[test]
    fn nothing_to_read_is_an_empty_history() {
        assert!(parse_history("").is_empty());
        assert!(parse_history("\n\n").is_empty());
    }

    /// The file cannot grow without bound on a machine used for years.
    #[test]
    fn the_entry_count_is_capped_on_read() {
        let contents: String = (0..MAX_ENTRIES + 500)
            .map(|n| format!("prompt {n}\n"))
            .collect();
        let entries = parse_history(&contents);
        assert_eq!(entries.len(), MAX_ENTRIES);
        // Kept from the end, since recall walks backwards from the newest.
        assert_eq!(
            entries.last().unwrap(),
            &format!("prompt {}", MAX_ENTRIES + 499)
        );
    }

    /// A pasted file is not worth storing, and recalling it would not be useful either.
    #[test]
    fn over_long_entries_are_dropped_on_read() {
        let huge = "x".repeat(MAX_ENTRY_BYTES + 1);
        let contents = format!("keep\n{huge}\nkeep too\n");
        assert_eq!(
            parse_history(&contents),
            vec!["keep".to_string(), "keep too".to_string()]
        );
    }

    /// A prompt exactly at the limit is kept, so the boundary is not off by one.
    #[test]
    fn an_entry_at_the_limit_is_kept() {
        let exact = "x".repeat(MAX_ENTRY_BYTES);
        assert_eq!(parse_history(&format!("{exact}\n")), vec![exact]);
    }
}
