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
