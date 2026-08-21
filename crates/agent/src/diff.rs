//! Line diffs, so a person reviewing a write sees the change rather than the result.
//!
//! A write approval is only meaningful if the reviewer can tell what would be lost. A
//! whole proposed body does not show that: the reader has to hold the old file in their
//! head and spot the difference. So the confirmation shows a diff, and the parts that did
//! not change are collapsed to keep the question on screen.
//!
//! Both sides arrive here as plain strings — they have already been through the gates and
//! released for display — so nothing in this module needs to reason about labels.

/// One line of a rendered diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Kept(String),
    Added(String),
    Removed(String),
    /// A run of unchanged lines left out of a condensed view.
    Elided(usize),
}

/// The difference between two texts, by line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    changes: Vec<Change>,
    added: usize,
    removed: usize,
    exact: bool,
}

/// Cells the longest-common-subsequence table may occupy.
///
/// The table is quadratic in the differing region, so a bound is needed: two large and
/// wholly dissimilar files would otherwise allocate without limit. Past this the diff
/// reports itself inexact and the caller falls back to describing the change in the
/// aggregate, which is honest about what it does not know.
const MAX_CELLS: usize = 1_000_000;

impl Diff {
    /// Diff `before` against `after`, by line.
    pub fn compute(before: &str, after: &str) -> Self {
        let old: Vec<&str> = before.lines().collect();
        let new: Vec<&str> = after.lines().collect();

        // Matching head and tail are trimmed first. Most edits touch a small region, and
        // this is what keeps the quadratic step small enough to run at all.
        let mut start = 0;
        while start < old.len() && start < new.len() && old[start] == new[start] {
            start += 1;
        }
        let mut old_end = old.len();
        let mut new_end = new.len();
        while old_end > start && new_end > start && old[old_end - 1] == new[new_end - 1] {
            old_end -= 1;
            new_end -= 1;
        }

        let old_middle = &old[start..old_end];
        let new_middle = &new[start..new_end];

        if old_middle.len().saturating_mul(new_middle.len()) > MAX_CELLS {
            return Self {
                changes: Vec::new(),
                added: new_middle.len(),
                removed: old_middle.len(),
                exact: false,
            };
        }

        let mut changes: Vec<Change> = old[..start]
            .iter()
            .map(|l| Change::Kept(l.to_string()))
            .collect();
        changes.extend(subsequence_changes(old_middle, new_middle));
        changes.extend(old[old_end..].iter().map(|l| Change::Kept(l.to_string())));

        let added = changes
            .iter()
            .filter(|c| matches!(c, Change::Added(_)))
            .count();
        let removed = changes
            .iter()
            .filter(|c| matches!(c, Change::Removed(_)))
            .count();

        Self {
            changes,
            added,
            removed,
            exact: true,
        }
    }

    /// Lines added.
    pub fn added(&self) -> usize {
        self.added
    }

    /// Lines removed.
    pub fn removed(&self) -> usize {
        self.removed
    }

    /// Whether the line-by-line changes were computed, or only counted.
    ///
    /// False for a difference too large to diff, where [`Diff::condensed`] is empty and
    /// only the counts are meaningful.
    pub fn is_exact(&self) -> bool {
        self.exact
    }

    /// Whether the two texts differ by line at all.
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0
    }

    /// The changed lines, with at most `context` unchanged lines around each run and
    /// longer unchanged stretches replaced by [`Change::Elided`].
    pub fn condensed(&self, context: usize) -> Vec<Change> {
        let keep: Vec<bool> = self
            .changes
            .iter()
            .enumerate()
            .map(|(index, change)| match change {
                Change::Kept(_) => self.near_a_change(index, context),
                _ => true,
            })
            .collect();

        let mut out = Vec::new();
        let mut skipped = 0usize;
        for (change, keeping) in self.changes.iter().zip(keep) {
            if keeping {
                if skipped > 0 {
                    out.push(Change::Elided(skipped));
                    skipped = 0;
                }
                out.push(change.clone());
            } else {
                skipped += 1;
            }
        }
        if skipped > 0 {
            out.push(Change::Elided(skipped));
        }
        out
    }

    fn near_a_change(&self, index: usize, context: usize) -> bool {
        let low = index.saturating_sub(context);
        let high = index
            .saturating_add(context)
            .min(self.changes.len().saturating_sub(1));
        (low..=high).any(|i| !matches!(self.changes[i], Change::Kept(_)))
    }
}

/// Walk a longest-common-subsequence table to turn two line runs into changes.
///
/// Removals are emitted before additions where the table is indifferent, so a replaced
/// line reads as the old text followed by the new.
fn subsequence_changes(old: &[&str], new: &[&str]) -> Vec<Change> {
    let (n, m) = (old.len(), new.len());
    if n == 0 {
        return new.iter().map(|l| Change::Added(l.to_string())).collect();
    }
    if m == 0 {
        return old.iter().map(|l| Change::Removed(l.to_string())).collect();
    }

    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if old[i] == new[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }

    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            out.push(Change::Kept(old[i].to_string()));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            out.push(Change::Removed(old[i].to_string()));
            i += 1;
        } else {
            out.push(Change::Added(new[j].to_string()));
            j += 1;
        }
    }
    out.extend(old[i..].iter().map(|l| Change::Removed(l.to_string())));
    out.extend(new[j..].iter().map(|l| Change::Added(l.to_string())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn added(diff: &Diff) -> Vec<String> {
        diff.condensed(usize::MAX)
            .into_iter()
            .filter_map(|c| match c {
                Change::Added(line) => Some(line),
                _ => None,
            })
            .collect()
    }

    fn removed(diff: &Diff) -> Vec<String> {
        diff.condensed(usize::MAX)
            .into_iter()
            .filter_map(|c| match c {
                Change::Removed(line) => Some(line),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn identical_text_has_no_changes() {
        let diff = Diff::compute("a\nb\n", "a\nb\n");
        assert!(diff.is_empty());
        assert_eq!(diff.added(), 0);
        assert_eq!(diff.removed(), 0);
    }

    #[test]
    fn a_replaced_line_shows_both_sides() {
        let diff = Diff::compute("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(removed(&diff), vec!["b"]);
        assert_eq!(added(&diff), vec!["B"]);
        assert_eq!((diff.added(), diff.removed()), (1, 1));
    }

    #[test]
    fn an_insertion_removes_nothing() {
        let diff = Diff::compute("a\nc\n", "a\nb\nc\n");
        assert_eq!(added(&diff), vec!["b"]);
        assert!(removed(&diff).is_empty());
    }

    #[test]
    fn a_deletion_adds_nothing() {
        let diff = Diff::compute("a\nb\nc\n", "a\nc\n");
        assert_eq!(removed(&diff), vec!["b"]);
        assert!(added(&diff).is_empty());
    }

    #[test]
    fn a_new_file_is_all_additions() {
        let diff = Diff::compute("", "a\nb\n");
        assert_eq!(added(&diff), vec!["a", "b"]);
        assert_eq!(diff.removed(), 0);
    }

    #[test]
    fn an_emptied_file_is_all_removals() {
        let diff = Diff::compute("a\nb\n", "");
        assert_eq!(removed(&diff), vec!["a", "b"]);
        assert_eq!(diff.added(), 0);
    }

    /// The point of the condensed view: a one-line change in a large file must not print
    /// the whole file.
    #[test]
    fn unchanged_stretches_are_elided() {
        let before: String = (0..100).map(|n| format!("line {n}\n")).collect();
        let after = before.replace("line 50\n", "changed\n");

        let condensed = Diff::compute(&before, &after).condensed(2);

        let kept = condensed
            .iter()
            .filter(|c| matches!(c, Change::Kept(_)))
            .count();
        assert_eq!(
            kept, 4,
            "expected two context lines either side: {condensed:?}"
        );
        assert!(
            condensed
                .iter()
                .any(|c| matches!(c, Change::Elided(n) if *n > 40)),
            "the unchanged bulk was not elided: {condensed:?}"
        );
    }

    #[test]
    fn eliding_reports_every_omitted_line() {
        let before: String = (0..50).map(|n| format!("line {n}\n")).collect();
        let after = before.replace("line 25\n", "changed\n");
        let diff = Diff::compute(&before, &after);

        let condensed = diff.condensed(1);
        let shown = condensed
            .iter()
            .filter(|c| !matches!(c, Change::Elided(_)))
            .count();
        let elided: usize = condensed
            .iter()
            .map(|c| match c {
                Change::Elided(n) => *n,
                _ => 0,
            })
            .sum();

        // Nothing may vanish: every line of the full diff is either shown or counted.
        assert_eq!(shown + elided, diff.changes.len());
    }

    /// A difference too large to diff must say so rather than silently reporting no
    /// changes, which would read as "this write changes nothing".
    #[test]
    fn an_oversized_difference_is_inexact_but_still_counted() {
        let before: String = (0..2000).map(|n| format!("old {n}\n")).collect();
        let after: String = (0..2000).map(|n| format!("new {n}\n")).collect();

        let diff = Diff::compute(&before, &after);
        assert!(!diff.is_exact());
        assert!(diff.condensed(3).is_empty());
        assert_eq!((diff.added(), diff.removed()), (2000, 2000));
        assert!(!diff.is_empty(), "an inexact diff still changed something");
    }

    /// Trimming the matching head and tail must not misplace the changed region.
    #[test]
    fn a_change_at_the_end_is_located_correctly() {
        let diff = Diff::compute("a\nb\nc\n", "a\nb\nz\n");
        assert_eq!(removed(&diff), vec!["c"]);
        assert_eq!(added(&diff), vec!["z"]);
    }

    #[test]
    fn a_change_at_the_start_is_located_correctly() {
        let diff = Diff::compute("a\nb\nc\n", "z\nb\nc\n");
        assert_eq!(removed(&diff), vec!["a"]);
        assert_eq!(added(&diff), vec!["z"]);
    }

    /// A repeated line is where a naive prefix/suffix trim goes wrong.
    #[test]
    fn repeated_lines_diff_correctly() {
        let diff = Diff::compute("x\nx\nx\n", "x\nx\nx\nx\n");
        assert_eq!(added(&diff), vec!["x"]);
        assert!(removed(&diff).is_empty());
    }
}
