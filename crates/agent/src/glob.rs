//! Glob matching for workspace paths.
//!
//! Listings and searches need narrowing, say "just the Rust files under crates", or a model
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
//! - `{a,b}` matches either alternative, and may be nested
//!
//! A pattern with no `/` matches against the file name alone, so `*.rs` finds Rust files at
//! any depth, which is the reading a person intends when they type it.
//!
//! Brace groups are expanded before matching rather than matched, which is what keeps the
//! guarantee above: each alternative is an ordinary pattern walked once, so a group costs a
//! multiple of the work and never a power of it. [`expand`] is separate from [`matches`]
//! because a walk applies one pattern to thousands of paths, and expanding once at the top
//! is the difference between one allocation and one per file.

/// How many patterns one expansion may produce.
///
/// Groups multiply: `{a,b}{c,d}{e,f}` is eight patterns and ten such groups are a thousand.
/// Past the cap the pattern is matched literally, which finds nothing and is *reported* as
/// finding nothing, rather than quietly becoming a walk nobody asked for.
const MAX_EXPANSIONS: usize = 64;

/// Whether `path` matches `pattern`.
///
/// `path` is expected to use `/` separators, as workspace-relative paths do.
///
/// Expands on every call. Use [`expand`] with [`matches_any`] where the same pattern is
/// applied to more than a handful of paths.
pub fn matches(pattern: &str, path: &str) -> bool {
    matches_any(&expand(pattern), path)
}

/// Whether `path` matches any of `patterns`, as produced by [`expand`].
pub fn matches_any(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|pattern| matches_single(pattern, path))
}

/// Expand brace groups into the plain patterns they stand for.
///
/// An empty pattern expands to nothing, so it matches nothing. A pattern with no group, an
/// unbalanced one, or one that would exceed [`MAX_EXPANSIONS`] comes back as itself: a
/// half-expanded pattern would match a set nobody wrote, and matching the literal is at
/// least a result the writer can recognise as wrong.
pub fn expand(pattern: &str) -> Vec<String> {
    if pattern.is_empty() {
        return Vec::new();
    }

    let mut done: Vec<String> = Vec::new();
    let mut pending: Vec<String> = vec![pattern.to_string()];

    while let Some(current) = pending.pop() {
        let Some((prefix, alternatives, suffix)) = split_first_group(&current) else {
            done.push(current);
            continue;
        };

        // Counted before anything is pushed, so the cap bounds what is built rather than
        // catching it afterwards.
        if done.len() + pending.len() + alternatives.len() > MAX_EXPANSIONS {
            return vec![pattern.to_string()];
        }

        // A nested group is left in place and handled when the result comes back around.
        for alternative in alternatives {
            pending.push(format!("{prefix}{alternative}{suffix}"));
        }
    }

    done
}

/// Split out the first brace group, as (before, alternatives, after).
///
/// `None` where there is no group or the braces do not balance. Nesting is counted rather
/// than assumed away, so the `}` that closes `{a,{b,c}}` is the last one and not the first.
fn split_first_group(pattern: &str) -> Option<(&str, Vec<&str>, &str)> {
    let open = pattern.find('{')?;

    let mut depth = 0usize;
    let mut close = None;
    // Byte indices, and both braces are ASCII, so slicing on them lands on a boundary.
    for (index, character) in pattern[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + index);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;

    let alternatives = split_alternatives(&pattern[open + 1..close]);
    Some((&pattern[..open], alternatives, &pattern[close + 1..]))
}

/// Split a group's interior on the commas that belong to it, ignoring nested ones.
///
/// `{a,{b,c}}` has two alternatives, not three: the inner comma is the inner group's.
fn split_alternatives(interior: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;

    for (index, character) in interior.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&interior[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    out.push(&interior[start..]);
    out
}

/// Whether `path` matches one already-expanded pattern.
fn matches_single(pattern: &str, path: &str) -> bool {
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

    /// The spelling everybody reaches for. It used to be matched literally, which found
    /// nothing and read as proof the files were absent.
    #[test]
    fn a_brace_group_matches_each_alternative() {
        assert!(matches("*.{ts,tsx}", "a.ts"));
        assert!(matches("*.{ts,tsx}", "a.tsx"));
        assert!(!matches("*.{ts,tsx}", "a.js"));
        assert!(matches("**/*.{cc,h,mm}", "ios/browser/policy/map.h"));
    }

    /// Groups multiply rather than add, and a group may sit anywhere in the pattern.
    #[test]
    fn groups_combine() {
        assert!(matches("{src,tests}/**/*.{rs,toml}", "src/a/b.rs"));
        assert!(matches("{src,tests}/**/*.{rs,toml}", "tests/Cargo.toml"));
        assert!(!matches("{src,tests}/**/*.{rs,toml}", "docs/a/b.rs"));
    }

    #[test]
    fn a_nested_group_expands_from_the_inside() {
        let mut expanded = expand("a{b,c{d,e}}f");
        expanded.sort();
        assert_eq!(expanded, ["abf", "acdf", "acef"]);
    }

    /// An alternative may be empty, which is how an optional piece is written.
    #[test]
    fn an_empty_alternative_stands_for_nothing() {
        assert!(matches("a{,-test}.rs", "a.rs"));
        assert!(matches("a{,-test}.rs", "a-test.rs"));
    }

    /// Half-expanding is worse than not expanding: the pattern would match a set nobody
    /// wrote. Matching the literal at least fails in a way its author can recognise.
    #[test]
    fn an_unbalanced_brace_is_matched_literally() {
        assert_eq!(expand("*.{ts"), ["*.{ts"]);
        assert!(matches("*.{ts", "a.{ts"));
        assert!(!matches("*.{ts", "a.ts"));
    }

    /// The cap is what stops a pattern from becoming a walk of its own.
    #[test]
    fn an_oversized_expansion_falls_back_to_the_literal() {
        // Seven groups of two is 128, past the cap of 64.
        let pattern = "{a,b}{a,b}{a,b}{a,b}{a,b}{a,b}{a,b}";
        assert_eq!(expand(pattern), [pattern]);

        // Six is 64, which fits, and is the boundary worth pinning.
        let ok = "{a,b}{a,b}{a,b}{a,b}{a,b}{a,b}";
        assert_eq!(expand(ok).len(), 64);
    }

    #[test]
    fn a_pattern_without_a_group_expands_to_itself() {
        assert_eq!(expand("**/*.rs"), ["**/*.rs"]);
        assert!(expand("").is_empty());
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
