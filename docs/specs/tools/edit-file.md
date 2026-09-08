---
id: EDIT
title: edit_file
status: normative
governs:
  - crates/agent/src/replace.rs
  - crates/agent/src/diff.rs
---

## Scope

Replacing an exact passage in a file. `path` and `replace_all` are routing; `old_text` and
`new_text` are content. The result is a confirmation.

## Why this exists rather than a whole-file write

Reviewing a whole file body on a terminal is not review. An edit names the exact passage, so a
person approves a diff of a few lines.

## Clauses

<a id="EDIT-1"></a>
### EDIT-1: an edit refuses rather than guesses

It refuses when the passage is missing, when it occurs more than once without `replace_all`, and
when the file changed since it was read.

**Why.** A guess would change bytes nobody reviewed, and a stale diff would not describe what
actually happens.

`verified-by: bravebot_agent::turn::an_ambiguous_edit_is_refused_without_asking`
`verified-by: bravebot_agent::turn::an_approved_edit_changes_only_the_matched_passage`

<a id="EDIT-2"></a>
### EDIT-2: an edit requires a trusted file

Locating a passage means comparing text, and a comparison is a decision. On a file nobody vouched
for that decision would be taken from bytes an attacker may have written, so it is refused rather
than performed. The route for such a file is a processor and a whole-file write, where nothing is
located and the body is shown to a person in full.

`verified-by: bravebot_agent::turn::editing_an_untrusted_file_is_refused`

<a id="EDIT-3"></a>
### EDIT-3: an edit is approved as a diff, and cannot leave the workspace

`verified-by: bravebot_agent::turn::an_edit_is_reviewed_as_a_diff`
`verified-by: bravebot_agent::turn::an_approved_edit_is_recorded_as_endorsed`
`verified-by: bravebot_agent::turn::a_refused_edit_does_not_happen`
`verified-by: bravebot_agent::turn::an_edit_cannot_escape_the_workspace`

<a id="EDIT-4"></a>
### EDIT-4: an edit comes back with the lines it produced

The result carries the changed region of the file, with a few lines either side and their line
numbers, in front of the count of replacements. The region is found by comparing the file before
and after rather than by locating the new text, so a deletion, an insertion and a `replace_all`
are all shown; a span longer than the excerpt allows is cut in the middle and says how much it
dropped.

**Why.** A count of replacements is not a result anybody can check. A session edited eighteen
files on nothing but those counts, never looked at one of them again, and compiled none of it. The
lines around a change answer, in the result the planner already has, the question it would
otherwise spend a round asking, or worse not ask at all.

**Nothing is declassified for it.** The excerpt is shaped inside the kernel and handed over
labelled, exactly as a read is, so `Policy::present` decides whether the planner sees it. Where the
label is untrusted the result is the count alone, because a quarantined excerpt would take the
confirmation down with it and a planner that cannot be told its edit landed is worse off than one
told only that. [EDIT-2](#EDIT-2) is what makes the file itself trusted by this point.

`verified-by: bravebot_agent::turn::an_edit_shows_the_lines_it_changed`
`verified-by: bravebot_agent::replace::an_excerpt_shows_the_changed_line_with_its_neighbours`
`verified-by: bravebot_agent::replace::a_long_span_is_cut_in_the_middle_and_says_so`
