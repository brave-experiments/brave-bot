//! Exact text replacement, and the reasons to refuse one.
//!
//! An edit names a passage to replace rather than a whole file, which is what makes its
//! approval reviewable. But naming a passage can be ambiguous, and an ambiguous edit
//! mutates bytes nobody chose. So the two ambiguous cases are refused outright:
//!
//! - the passage is not in the file, so the model is working from a stale or imagined read;
//! - the passage occurs more than once, so which one was meant is unknowable.
//!
//! Refusing is safe in a way that guessing is not: a refusal costs a step and tells the
//! model what to fix, while a wrong guess destroys work the user did not review.
//!
//! Matching is exact and byte-for-byte. Fuzzy correction — trimming whitespace, re-indenting
//! to fit — is deliberately absent: it turns "this is what I am replacing" into a guess, and
//! the guess is the part that would not be shown to the reviewer.

use std::fmt;

/// Why a replacement was not performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceError {
    /// The text to replace was not found.
    NotFound,
    /// The text occurs more than once and no instruction covered which to change.
    Ambiguous { occurrences: usize },
    /// The old and new text are the same, so there is nothing to do.
    Unchanged,
    /// The text to replace was empty, which matches everywhere and nowhere usefully.
    EmptyPattern,
}

impl fmt::Display for ReplaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(
                f,
                "the text to replace is not in the file; it must match exactly, including \
                 whitespace and indentation. Read the file again to see its current contents"
            ),
            Self::Ambiguous { occurrences } => write!(
                f,
                "the text to replace occurs {occurrences} times, so it is unclear which was \
                 meant. Include more surrounding context to make it unique, or set \
                 replace_all to true to change every occurrence"
            ),
            Self::Unchanged => write!(
                f,
                "the old and new text are identical, so this edit would change nothing"
            ),
            Self::EmptyPattern => write!(
                f,
                "the text to replace must not be empty; use write_file to create or replace \
                 a whole file"
            ),
        }
    }
}

/// The result of applying an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replaced {
    /// The full resulting text.
    pub contents: String,
    /// How many occurrences were replaced.
    pub occurrences: usize,
}

/// Replace `old` with `new` in `source`.
///
/// With `all` false, refuses unless exactly one occurrence exists.
pub fn replace(source: &str, old: &str, new: &str, all: bool) -> Result<Replaced, ReplaceError> {
    if old.is_empty() {
        return Err(ReplaceError::EmptyPattern);
    }
    if old == new {
        return Err(ReplaceError::Unchanged);
    }

    let occurrences = source.matches(old).count();
    match occurrences {
        0 => Err(ReplaceError::NotFound),
        1 => Ok(Replaced {
            contents: source.replacen(old, new, 1),
            occurrences: 1,
        }),
        many if all => Ok(Replaced {
            contents: source.replace(old, new),
            occurrences: many,
        }),
        many => Err(ReplaceError::Ambiguous { occurrences: many }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unique_match_is_replaced() {
        let result = replace("a\nb\nc\n", "b", "B", false).expect("unique");
        assert_eq!(result.contents, "a\nB\nc\n");
        assert_eq!(result.occurrences, 1);
    }

    /// The central refusal: guessing which of several matches was meant would mutate
    /// bytes the reviewer never chose.
    #[test]
    fn several_matches_are_refused_by_default() {
        let error = replace("x\nx\n", "x", "y", false).expect_err("ambiguous");
        assert_eq!(error, ReplaceError::Ambiguous { occurrences: 2 });
    }

    #[test]
    fn several_matches_are_replaced_when_asked() {
        let result = replace("x\nx\nx\n", "x", "y", true).expect("all");
        assert_eq!(result.contents, "y\ny\ny\n");
        assert_eq!(result.occurrences, 3);
    }

    /// A missing match means the model is working from a stale read, so it must be told
    /// rather than have the edit silently do nothing.
    #[test]
    fn a_missing_match_is_refused() {
        assert_eq!(
            replace("a\n", "zzz", "y", false).expect_err("absent"),
            ReplaceError::NotFound
        );
    }

    #[test]
    fn an_empty_pattern_is_refused() {
        assert_eq!(
            replace("a\n", "", "y", false).expect_err("empty"),
            ReplaceError::EmptyPattern
        );
    }

    #[test]
    fn an_identical_replacement_is_refused() {
        assert_eq!(
            replace("a\n", "a", "a", false).expect_err("identical"),
            ReplaceError::Unchanged
        );
    }

    /// Whitespace is part of the match, so a passage indented differently from the file is
    /// not found rather than quietly re-indented to fit.
    #[test]
    fn matching_is_whitespace_exact() {
        let source = "if (x) {\n    y();\n}\n";
        assert_eq!(
            replace(source, "if (x) {\n  y();\n}", "z", false).expect_err("indentation differs"),
            ReplaceError::NotFound
        );
        let result = replace(source, "if (x) {\n    y();\n}", "z", false).expect("exact match");
        assert_eq!(result.contents, "z\n");
    }

    /// Replacing with a multi-line body is how an edit inserts code.
    #[test]
    fn a_replacement_may_span_lines() {
        let result = replace("a\nb\n", "b", "b1\nb2", false).expect("multi-line");
        assert_eq!(result.contents, "a\nb1\nb2\n");
    }

    /// Only the first occurrence is replaced in the single case, and the count reflects
    /// that — an overlapping pattern must not be double-counted into ambiguity.
    #[test]
    fn a_match_spanning_lines_is_found() {
        let result = replace("one\ntwo\nthree\n", "one\ntwo", "1\n2", false).expect("spanning");
        assert_eq!(result.contents, "1\n2\nthree\n");
    }

    /// The error text must tell the model what to do next, since it is what the model
    /// reads to recover.
    #[test]
    fn refusals_explain_the_remedy() {
        assert!(
            ReplaceError::Ambiguous { occurrences: 2 }
                .to_string()
                .contains("replace_all")
        );
        assert!(ReplaceError::NotFound.to_string().contains("Read the file"));
        assert!(
            ReplaceError::EmptyPattern
                .to_string()
                .contains("write_file")
        );
    }
}
