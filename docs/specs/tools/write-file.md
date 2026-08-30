---
id: WRITE
title: write_file
status: normative
governs:
  - crates/agent/src/workspace.rs
---

## Scope

Writing a whole file. `path` and `contents_ref` are routing; `contents` is content. The result is a
confirmation.

## Clauses

<a id="WRITE-1"></a>
### WRITE-1: contents or a reference, never both

`contents_ref` names quarantined content that becomes the whole file. It is **routing**, since it
decides which bytes the write carries, and it is a name the driver handed out rather than anything
derived from content.

**Why.** The worst a wrong reference can do is put the wrong quarantined bytes into a path that
still had to be endorsed on its own.

`verified-by: bravebot_agent::turn::a_quarantined_file_is_rewritten_by_a_processor`

<a id="WRITE-2"></a>
### WRITE-2: a reference that names no file is not a destination

Everything a processor produced is such a reference. That refusal is what stops untrusted text
choosing where an effect lands.

`verified-by: bravebot_agent::turn::a_processors_output_cannot_be_a_destination`
`verified-by: bravebot_core::policy::a_reference_that_names_no_file_is_not_a_destination`

<a id="WRITE-3"></a>
### WRITE-3: the planner never chooses a destination on its own

A write needs a person's approval, which mints a single-use endorsement bound to that exact path,
so it cannot be replayed or redirected. Where nobody can be asked, writes are refused rather than
applied unseen.

**Why.** The wrong file destroys work rather than wasting a step.

`verified-by: bravebot_agent::turn::an_approved_write_is_recorded_as_endorsed`
`verified-by: bravebot_agent::turn::a_refused_write_does_not_happen`
`verified-by: bravebot_agent::turn::a_refused_overwrite_leaves_the_original`
`verified-by: bravebot_agent::turn::an_approved_write_cannot_escape_the_workspace`

<a id="WRITE-4"></a>
### WRITE-4: a write through a reference is always shown

Even where the trust map would not ask. The person approving is shown the path.

**Why.** The approval is the only moment the filename is visible to anybody. They own the
directory and are the only party who can say whether that file should be rewritten.

`verified-by: bravebot_agent::turn::every_write_through_a_reference_is_shown`
`verified-by: bravebot_agent::turn::a_write_through_a_reference_says_what_landed_and_that_it_is_done`
`verified-by: bravebot_agent::turn::a_reference_write_is_reviewed_as_a_diff`
