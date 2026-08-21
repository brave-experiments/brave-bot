//! Glob matching for workspace paths.
//!
//! Listings and searches need narrowing — "just the Rust files under crates" — or a model
//! spends a step reading a listing it mostly does not want.
//!
//! A glob rather than a regular expression, and hand-written rather than a dependency, for
//! the same reason `search` matches literally: patterns arrive through a turn, and a
//! backtracking engine turns a pattern into a denial-of-service vector. The matcher below
//! runs in time proportional to the path length times the pattern length, with no
//! backtracking and no recursion, so a hostile pattern costs nothing unusual.
//!
//! Supported, and nothing else:
//!
//! - `?` matches one character within a segment
//! - `*` matches any run of characters within a segment
//! - `**` matches across segment boundaries
//!
//! A pattern with no `/` matches against the file name alone, so `*.rs` finds Rust files at
//! any depth — the reading a person intends when they type it.

/// Whether `path` matches `pattern`.
///
/// `path` is expected to use `/` separators, as workspace-relative paths do.
pub fn matches(pattern: &str, path: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }

    // A bare name pattern is about the file, not its location. Without this, `*.rs` would
    // match nothing in a tree of any depth, which is never what a person means by it.
    let subject = if pattern.contains('/') {
        path
    } else {
        path.rsplit('/').next().unwrap_or(path)
    };

    matches_segments(pattern, subject)
}

/// Match segment by segment, so `**` is the only thing that can cross a `/`.
///
/// Matching per segment rather than over the whole string is what keeps the two kinds of
/// wildcard from interfering: a single `*` never sees a separator to begin with, so it
/// cannot be tempted across one, and `**` is resolved at the segment level where its
/// meaning is defined.
fn matches_segments(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();

    let (mut pi, mut si) = (0usize, 0usize);
    // Where to resume if the current `**` guess turns out to be too short.
    let mut star: Option<(usize, usize)> = None;

    while si < path.len() {
        if pi < pattern.len() && pattern[pi] == "**" {
            // Try consuming nothing first, and remember to consume more on failure.
            star = Some((pi, si));
            pi += 1;
            continue;
        }

        if pi < pattern.len() && matches_one(pattern[pi], path[si]) {
            pi += 1;
            si += 1;
            continue;
        }

        match star {
            Some((star_pi, star_si)) => {
                // Let the `**` swallow one more segment and retry from just after it.
                star = Some((star_pi, star_si + 1));
                pi = star_pi + 1;
                si = star_si + 1;
            }
            None => return false,
        }
    }

    // A trailing `**` may match no segments at all.
    while pi < pattern.len() && pattern[pi] == "**" {
        pi += 1;
    }

    pi == pattern.len()
}

/// Match one path segment against one pattern segment, where `*` and `?` cannot escape it.
///
/// The two-pointer walk resumes after the last `*` on a mismatch rather than branching, so
/// a pattern crafted to backtrack stays linear in the product of the lengths.
fn matches_one(pattern: &str, segment: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = segment.chars().collect();

    let (mut pi, mut si) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;

    while si < s.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi, si));
            pi += 1;
            continue;
        }
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
            continue;
        }
        match star {
            Some((star_pi, star_si)) => {
                star = Some((star_pi, star_si + 1));
                pi = star_pi + 1;
                si = star_si + 1;
            }
            None => return false,
        }
    }

    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }

    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_name_matches_itself() {
        assert!(matches("main.rs", "main.rs"));
        assert!(!matches("main.rs", "other.rs"));
    }

    /// The reading a person intends: a bare pattern is about the file name, so it finds
    /// matches at any depth.
    #[test]
    fn a_bare_pattern_matches_at_any_depth() {
        assert!(matches("*.rs", "main.rs"));
        assert!(matches("*.rs", "crates/agent/src/tools.rs"));
        assert!(!matches("*.rs", "crates/agent/Cargo.toml"));
    }

    /// A pattern with a separator is about the path, so it anchors.
    #[test]
    fn a_path_pattern_anchors_at_the_root() {
        assert!(matches("src/*.rs", "src/main.rs"));
        assert!(!matches("src/*.rs", "other/main.rs"));
        // A single star must not leap a directory boundary.
        assert!(!matches("src/*.rs", "src/deep/main.rs"));
    }

    #[test]
    fn a_double_star_crosses_directories() {
        assert!(matches("src/**/*.rs", "src/a/b/c.rs"));
        assert!(matches("**/*.rs", "a/b/c.rs"));
        assert!(matches("crates/**/tools.rs", "crates/agent/src/tools.rs"));
    }

    /// `**` must also match no directories at all, or `src/**/x.rs` surprises by missing
    /// `src/x.rs`.
    #[test]
    fn a_double_star_matches_nothing_too() {
        assert!(matches("src/**/x.rs", "src/x.rs"));
        assert!(matches("**/x.rs", "x.rs"));
    }

    /// A trailing `**` names a subtree.
    #[test]
    fn a_trailing_double_star_matches_a_subtree() {
        assert!(matches("src/**", "src/a/b.rs"));
        assert!(matches("src/**", "src/a.rs"));
        assert!(!matches("src/**", "other/a.rs"));
    }

    #[test]
    fn a_question_mark_matches_one_character() {
        assert!(matches("a?.rs", "ab.rs"));
        assert!(!matches("a?.rs", "abc.rs"));
        // But never a separator, which would let it escape its segment.
        assert!(!matches("a?b", "a/b"));
    }

    #[test]
    fn a_brace_free_extension_group_is_not_special() {
        // Braces are unsupported and treated literally rather than half-implemented.
        assert!(!matches("*.{ts,tsx}", "a.ts"));
        assert!(matches("*.{ts,tsx}", "a.{ts,tsx}"));
    }

    #[test]
    fn an_empty_pattern_matches_nothing() {
        assert!(!matches("", "a.rs"));
        assert!(!matches("", ""));
    }

    #[test]
    fn a_star_matches_an_empty_run() {
        assert!(matches("a*", "a"));
        assert!(matches("*a", "a"));
        assert!(matches("*", "anything"));
    }

    /// The property that makes this safe to expose: a pattern built to cause backtracking
    /// must still return promptly. Without the two-pointer walk this is exponential.
    #[test]
    fn a_pathological_pattern_does_not_blow_up() {
        let pattern = "*a*a*a*a*a*a*a*a*a*a*a*a*a*a*a*a*b";
        let path = "a".repeat(2_000);
        assert!(!matches(pattern, &path));
    }

    #[test]
    fn matching_is_case_sensitive() {
        assert!(!matches("*.RS", "main.rs"));
        assert!(matches("*.rs", "main.rs"));
    }
}
