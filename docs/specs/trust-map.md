---
id: TRUST
title: The trust map
status: normative
governs:
  - crates/core/src/trust.rs
  - crates/core/src/policy.rs
  - crates/tui/src/trust_prompt.rs
  - crates/tui/src/sessions.rs
  - crates/tui/src/dropped.rs
  - crates/agent/src/workspace.rs
guards:
  - symbol: TrustStore::trust
  - symbol: TrustStore::distrust
  - symbol: TrustStore::rebased
  - symbol: Policy::reconcile_after_write
  - symbol: Policy::vouch_for_named_path
---

## Scope

Which paths a user has vouched for, every way a rule enters that record, what a write does to
it, and how long an answer lasts. It does not cover what a label means once assigned, which is
[labels.md](labels.md), nor what may be released to whom, which is [routing.md](routing.md), nor
`~/.bravebot`, which the map does not govern (TRUST-11).

## The record

<a id="TRUST-1"></a>
### TRUST-1: nothing is trusted until it is granted

An empty map trusts no path. Trust is granted by a person and never inferred from silence, from
a path's shape, or from anything a model or a file said.

**Why.** This is what makes declining at startup mean something. A default of trusted would make
the answer decorative.

`verified-by: bravebot_core::trust::an_empty_store_trusts_nothing`

<a id="TRUST-2"></a>
### TRUST-2: the longest matching prefix decides

Rules are keyed by path prefix and matched by whole segments. Both polarities are expressible, so
a trusted tree may hold an untrusted subtree, which may hold a trusted path again. Equivalent
spellings of a path are one rule, and a later decision replaces an earlier one.

A rule is about a **path**, not about the files that were in it when the rule was made, and it is
consulted when a file is read rather than when the rule is written. A file that appears in a
trusted directory afterwards is therefore read as trusted, whoever put it there.

**Why.** Per-file exceptions in both directions are the only way `@vendor/lib.js` can be trusted
inside a `vendor` a person marked untrusted, without that answer leaking to its siblings.

`verified-by: bravebot_core::trust::the_deepest_rule_wins_at_any_depth`
`verified-by: bravebot_core::trust::an_untrusted_subpath_overrides_a_trusted_parent`
`verified-by: bravebot_core::trust::a_trusted_subpath_overrides_an_untrusted_parent`
`verified-by: bravebot_core::trust::a_rule_matches_whole_segments_only`
`verified-by: bravebot_core::trust::equivalent_path_spellings_are_the_same_rule`
`verified-by: bravebot_core::trust::every_equivalent_spelling_of_a_path_reaches_the_same_rule`
`verified-by: bravebot_agent::workspace::a_second_spelling_of_a_distrusted_file_is_read_as_untrusted`
`verified-by: bravebot_core::trust::a_later_decision_replaces_an_earlier_one`

<a id="TRUST-3"></a>
### TRUST-3: relative and absolute rules are separate namespaces

Keeping two namespaces is a workaround rather than a preference, and
[issue #24](https://github.com/brave-experiments/bravebot/issues/24) proposes replacing both with
one map of full paths, which would remove this clause.

A rule under the working directory decides nothing about a directory opened by absolute path, and
the reverse. `/` is never treated as the empty prefix.

**Why.** The working directory's own rule is the **empty** prefix, since every path in the project
is named relative to it. Match absolute paths against that same map and the empty prefix covers
every one of them, so answering yes at startup would silently vouch for every directory opened
later and for anything else named by absolute path. The reverse holds too: a rule on `/` would
cover every relative path in the project. Keeping the two apart is what stops either answer
reaching where it was never given.

The same relative path also exists in both places and names different files, so even without the
prefix problem one map could not tell them apart.

`verified-by: bravebot_core::trust::an_absolute_rule_does_not_decide_a_relative_path`
`verified-by: bravebot_core::trust::trusting_the_workspace_says_nothing_about_an_added_directory`
`verified-by: bravebot_core::trust::trusting_the_filesystem_root_does_not_trust_the_workspace`
`verified-by: bravebot_core::trust::one_added_directory_does_not_cover_a_sibling`
`verified-by: bravebot_core::trust::the_deepest_absolute_rule_wins`
`verified-by: bravebot_core::trust::equivalent_absolute_spellings_are_the_same_rule`
`verified-by: bravebot_core::trust::every_equivalent_absolute_spelling_reaches_the_same_rule`

## What a write does

<a id="TRUST-4"></a>
### TRUST-4: what a write asks, and what it records

Every row is normative. A write matching a row does exactly what that row says and nothing else.

| data | destination | prompt? | effect on the map |
|---|---|---|---|
| trusted | trusted | no | unchanged |
| untrusted | trusted | **yes** | that path becomes untrusted |
| trusted | untrusted | no | that path becomes trusted |
| untrusted | untrusted | no | unchanged |
| either | never mentioned | **yes** | that path takes the data's trust |

A prompt asks one question and only this one: **may this path stop being trusted?** That is the
only consequence a later step cannot undo, since a path recorded as untrusted can no longer be
examined or edited.

**Why writing trusted data never asks.** Trusted data means the turn observed nothing untrusted,
so it holds no byte an attacker influenced, and the destination only ever gains trust. There is
nothing to ask about.

**Why untrusted data into a trusted path must mark it untrusted.** This closes the round trip.
Untrusted bytes are anything derived from the web or from a file outside a trusted path; written
into a trusted tree and read back as trusted they would launder injected text into trusted input,
and the map would become a bypass for the gate it exists to support.

**Why a path nobody has mentioned asks either way.** It differs from one deliberately marked
untrusted: the first has no decision behind it, so the first write there is the moment to ask.
This is also what makes declining at startup meaningful, since with nothing vouched for every
write is shown.

`verified-by: bravebot_core::policy::trusted_data_into_a_trusted_path_is_silent_and_changes_nothing`
`verified-by: bravebot_core::policy::untrusted_data_into_a_trusted_path_prompts_and_distrusts_the_path`
`verified-by: bravebot_core::policy::trusted_data_into_an_untrusted_path_is_silent_and_trusts_the_path`
`verified-by: bravebot_core::policy::untrusted_data_into_an_untrusted_path_is_silent_and_changes_nothing`
`verified-by: bravebot_core::policy::an_unvouched_path_prompts_either_way`
`verified-by: bravebot_core::policy::a_file_written_with_untrusted_data_reads_back_untrusted`
`verified-by: bravebot_core::policy::a_file_read_back_under_another_spelling_is_still_untrusted`

<a id="TRUST-5"></a>
### TRUST-5: reconciliation marks the exact path, never the parent

Reconciliation records the file written, and no directory above it.

**Why.** One untrusted file does not taint its siblings. Marking the parent would turn a single
fetched page into a project nobody may edit.

`verified-by: bravebot_core::policy::untrusted_data_into_a_trusted_path_prompts_and_distrusts_the_path`
`verified-by: bravebot_core::policy::trusted_data_into_an_untrusted_path_is_silent_and_trusts_the_path`

## How long an answer lasts

<a id="TRUST-6"></a>
### TRUST-6: the map belongs to the session, not the directory

Every session start asks, whatever any earlier session in that directory answered. `/clear` begins
a session and therefore asks. `--resume` does not ask, and restores the map from the record of the
session chosen; a record from before maps were kept has none, and is asked about.

**Why.** The question grants standing permission. Honouring last week's answer grants it on behalf
of a user who was never asked, and trust assumed from silence is not trust granted. A resume is not
an exception: the answer honoured is the one that session's own user gave, and it carries the rules
that session's writes recorded, which is what stops a resumed turn reading back a file an earlier
turn of the same session poisoned.

`verified-by: bravebot_tui::app::a_fresh_session_is_asked_rather_than_inheriting_a_map`
`verified-by: bravebot_tui::app::a_resume_starts_with_the_map_its_own_record_kept`
`verified-by: bravebot_tui::app::a_record_from_before_maps_were_kept_is_asked_about`
`verified-by: bravebot_tui::sessions::a_record_that_predates_the_map_has_none_rather_than_an_empty_one`
`verified-by: bravebot_tui::sessions::a_distrusted_path_inside_a_trusted_tree_survives_the_record`
`verified-by: bravebot_tui::sessions::two_recorded_spellings_of_one_path_resume_as_untrusted`
`verified-by: bravebot_tui::sessions::sessions_are_written_read_back_and_kept_per_directory`

<a id="TRUST-7"></a>
### TRUST-7: the startup question covers the whole workspace, and declining trusts nothing

At startup the user is asked whether they trust the working directory. Yes writes a rule covering
the tree. Declining writes nothing, so every write is shown. Leaving at the question starts no
session.

A session running in the mode that asks about nothing is the one exception: the question is not put,
and the map is the one a yes would have written. That mode approves vouching for every quarantined
file the planner reads, so the tree becomes trusted a file at a time whether or not the question is
asked, and a modal box is the most conspicuous thing there is to put to somebody who asked to be
asked about nothing. [permission-modes.md](permission-modes.md) is what selects that mode, and no
other mode may answer this question.

**Why the exception goes no further.** A resumed session takes the map from its own record even
there, since the question is not being put in that case either and the answer its user gave is the
more specific record.

`verified-by: bravebot_tui::trust_prompt::trusting_covers_the_whole_workspace`
`verified-by: bravebot_tui::trust_prompt::only_y_trusts_and_enter_answers_nothing`
`verified-by: bravebot_tui::trust_prompt::declining_trusts_nothing`
`verified-by: bravebot_tui::trust_prompt::leaving_starts_no_session`
`verified-by: bravebot_tui::trust_prompt::ctrl_c_leaves_rather_than_answering_the_question`
`verified-by: bravebot_tui::trust_prompt::bypassing_trusts_the_workspace_instead_of_asking`
`verified-by: bravebot_tui::trust_prompt::every_other_mode_leaves_the_question_to_the_person`
`verified-by: bravebot_tui::app::a_resume_keeps_its_own_map_even_where_the_mode_would_answer`

## The ways a rule is written

Each grants exactly one thing and grants it because a person made a gesture, never because
anything inspected content. TRUST-7 is the first. Three more write a rule the same way and have
specs of their own: naming a file is [naming-files.md](naming-files.md), dropping one on the
window is [dropping.md](dropping.md), and accepting a directory a settings file asked for is
[permissions.md](permissions.md). A file naming that directory writes nothing by itself.

<a id="TRUST-8"></a>
### TRUST-8: a quarantined read offers the same rule, at the moment it bites

When a turn reads a file nobody has vouched for, the user is shown the path and the first lines of
it and asked whether to trust it. Yes writes exactly the rule `@` would have written. Asked once
per path per turn, and only where the read is quarantined. Declining leaves the file as it was and
the turn carries on with a reference.

What the file holds decides nothing about whether the question is put. A file with nothing to show,
because it is empty or does not read as text, is asked about like any other, and the prompt says so
where the preview would be.

What does decide it is the path, and the question is put only where the path names a file. Never a
picture, because what a yes grants is that a file's text may be read and a picture's text is never
read whatever the map says. Never a directory or a path that names nothing either: a yes writes a
rule covering everything beneath the name it was given, so a prompt titled with one file would hand
over the trust half of what [`/add-dir`](#TRUST-9) grants, every file beneath the name at once, over
a string the planner chose rather than one a person typed. Reach it would not grant, which is the
only part of that gesture this could not have imitated.

```
╭ let the model read this file? ────────────────────────────╮
│Trust game.js                                              │
│                                                           │
│  the model cannot read this file, so it is working blind  │
│  on it. Vouching lets it read this file for the rest of   │
│  this session, here and in every later read.              │
│                                                           │
│┃ const SPEED = 100;                                       │
│                                                           │
│  y trust it    n leave it quarantined    ctrl-c stop      │
╰───────────────────────────────────────────────────────────╯
```

**Why.** This is the map's own decision offered where it matters, not a second route to trusting
content, so a yes stays consistent for every later read. It exists because of a session that did
not have it: asked to fix a bug in a game it could not read, the model pointed an isolated
processor at the file, wrote the answer back unseen, and finished by saying it could not confirm
any of what it had done. One prompt would have let it read the file.

`verified-by: bravebot_agent::turn::a_quarantined_read_offers_the_user_the_chance_to_vouch`
`verified-by: bravebot_agent::turn::a_quarantined_file_with_nothing_in_it_is_still_offered_for_vouching`
`verified-by: bravebot_agent::turn::a_picture_is_not_offered_for_vouching`
`verified-by: bravebot_agent::turn::a_path_that_names_no_file_is_not_offered_for_vouching`
`verified-by: bravebot_agent::turn::declining_to_vouch_leaves_the_file_quarantined`
`verified-by: bravebot_agent::turn::a_trusted_file_is_not_offered_for_vouching`
`verified-by: bravebot_agent::turn::the_same_file_is_offered_once_per_turn`
`verified-by: bravebot_tui::confirm::a_preview_with_nothing_in_it_says_so`

<a id="TRUST-9"></a>
### TRUST-9: `/add-dir` makes a directory both reachable and trusted, for the session

`/add-dir ~/notes` records an absolute rule (TRUST-3) that does two things together: the directory
becomes reachable, since an absolute path is otherwise refused whatever the map says, and it is
recorded as trusted. It lasts the session, `--resume` carries both halves, and `/clear` closes it.
A directory already inside the project is refused. A directory a resume cannot open again, because
it has moved or gone, is said so rather than passed over.

**Why.** Either half alone is no use, one leaving a rule about files nothing can open and the other
leaving a directory that prompts on every edit. It closes with the session for the reason every
other answer here does (TRUST-6): leaving a tree reachable once nothing vouches for it would
outlive the answer that allowed it.

`verified-by: bravebot_agent::workspace::a_file_in_an_added_directory_is_readable_by_its_absolute_path`
`verified-by: bravebot_agent::workspace::closing_added_directories_makes_them_unreachable_again`
`verified-by: bravebot_agent::workspace::a_new_file_can_be_created_in_an_added_directory`
`verified-by: bravebot_agent::turn::a_turn_can_read_a_file_in_an_added_directory`
`verified-by: bravebot_tui::sessions::a_resumed_session_can_still_open_the_directory_it_added`
`verified-by: bravebot_tui::sessions::a_directory_that_has_gone_since_is_reported_on_resume`

## Boundaries

<a id="TRUST-10"></a>
### TRUST-10: no rule extends reach; reading, writing and listing stay confined

Reading, writing, editing, listing and searching are confined to the working directory and to
whatever has been opened beside it. `..` and an absolute path outside those are refused rather than
resolved, in an added directory exactly as in the project, and a symlink leaving one is refused.
A relative path always means the project, so no file has two spellings. Naming a directory
includes nothing, since a directory is somewhere to type through rather than a file to read.

Confinement is decided by where an operation lands and not by how its path is spelled, so it holds
for a file that does not exist yet: a write creates what it names, and a symlink out of the tree is
refused whether or not there is anything at the other end of it.

**Why.** A directory symlink is an ordinary entry in a repository, so confinement that tested only
existing paths would let a checkout choose where a write lands. A person approved a path and the
prompt showed that path, so the bytes go there.

`verified-by: bravebot_agent::workspace::an_absolute_path_outside_every_added_directory_is_still_refused`
`verified-by: bravebot_agent::workspace::a_parent_component_cannot_climb_out_of_an_added_directory`
`verified-by: bravebot_agent::workspace::a_symlink_out_of_an_added_directory_is_refused`
`verified-by: bravebot_agent::workspace::creating_a_file_through_a_symlinked_directory_out_of_the_workspace_is_refused`
`verified-by: bravebot_agent::workspace::creating_a_file_through_a_symlinked_directory_in_an_added_directory_is_refused`
`verified-by: bravebot_agent::workspace::writing_to_a_dangling_symlink_out_of_the_workspace_is_refused`
`verified-by: bravebot_agent::workspace::overwriting_a_file_through_a_symlink_out_of_the_workspace_is_refused`
`verified-by: bravebot_agent::workspace::creating_a_file_through_a_symlink_inside_the_workspace_returns_where_it_landed`
`verified-by: bravebot_agent::workspace::writing_to_a_dangling_symlink_inside_the_workspace_lands_at_its_target`
`verified-by: bravebot_agent::workspace::a_destination_reached_through_a_dangling_symlink_out_of_the_workspace_is_refused`

<a id="TRUST-11"></a>
### TRUST-11: the map does not govern `~/.bravebot`

The user's own directory is read as trusted by provenance rather than by any rule here. A
project's own files are **not** covered by that and are read through this spec, whatever their
names. What is kept in that directory and how it is found is
[instructions.md](instructions.md); what it is trusted for is [skills.md](skills.md).

**Why.** The map is keyed by workspace-relative paths and has nothing to say about a path outside
the workspace. Asking it about one would be laundering.

`verified-by: bravebot_agent::skills::a_skill_the_trust_map_distrusts_stops_being_offered`

<a id="TRUST-12"></a>
### TRUST-12: `/status` lists the rules in force

Every rule the session holds is readable back, so what a line vouched for does not have to be
remembered.

`verified-by: bravebot_tui::status::an_added_directory_is_reported`
`verified-by: bravebot_tui::status::every_trust_rule_is_listed_however_many_there_are`

## Moving the working directory

<a id="TRUST-13"></a>
### TRUST-13: `/cd` moves the working directory, and the map moves with it

`/cd ~/projects/other` makes that directory the working directory: it is what a relative path
means from then on, where commands run, and where the project's own instructions are looked for.
The directory left behind closes, and so does any directory opened by name that holds the new one
or sits inside it, each said out loud as it happens.

The map does not travel unchanged. Every rule is re-spelled to say what it always said about the
same files: a rule inside the new working directory becomes a rule relative to it, and a rule
outside becomes an absolute one. Only then is the new directory vouched for, on the same footing
as `/add-dir`: the person typed the path, and a later decision replaces an earlier one.

**Why.** A relative rule means a path under the working directory, so a working directory that
moved without them would leave every one of them pointing at a file nobody decided anything about:
the yes given for one project would vouch for another, and every no given inside the old one would
be forgotten. Re-spelling grants and withdraws nothing, which is what makes it something this can
do without asking.

Nothing may overlap the new working directory, and that is not tidiness. A file reachable both
relatively and by absolute path has one rule in each namespace, and the two namespaces are kept
apart (TRUST-3) precisely so that one file has one answer.

`verified-by: bravebot_core::trust::a_yes_for_one_project_does_not_follow_a_move_to_another`
`verified-by: bravebot_core::trust::a_no_inside_the_new_directory_survives_the_move`
`verified-by: bravebot_core::trust::a_rule_in_an_added_directory_becomes_relative_when_it_is_moved_into`
`verified-by: bravebot_core::trust::rebasing_neither_grants_nor_withdraws_anything`
`verified-by: bravebot_agent::workspace::a_relative_path_means_the_new_working_directory_once_it_has_moved`
`verified-by: bravebot_agent::workspace::moving_closes_the_directory_left_behind`
`verified-by: bravebot_agent::workspace::moving_closes_an_added_directory_that_overlaps_the_new_one`
`verified-by: bravebot_agent::workspace::moving_leaves_an_unrelated_added_directory_open`
`verified-by: bravebot_tui::app::changing_directory_moves_the_workspace_and_vouches_for_where_it_moved`
`verified-by: bravebot_tui::app::changing_directory_leaves_the_previous_answer_where_it_was_given`
`verified-by: bravebot_tui::app::moving_into_a_directory_keeps_the_answers_given_inside_it`

## Known costs

Accepted deliberately. Do not "fix" one without changing this spec first.

- **A fresh session forgets what an earlier one poisoned.** The rule that untrusted data marks
  its destination untrusted holds within a session and
  across a resume of it. Across a fresh start it cannot, because the map it was recorded in is
  gone, so a file one session marked untrusted is read as trusted by the next session that vouches
  for the directory. The alternative is a per-directory map, which is a directory that trusts
  itself. If a file holds content you do not trust, the answer is to say no to the directory, or
  to not leave it there.
- **A file another process drops into a trusted directory is trusted.** TRUST-2 makes the rule
  about the path, so `npm install`, `git pull`, an editor, a background daemon, or a program the
  agent was allowed to run can all put a file inside a vouched-for tree and it will be read as
  trusted. TRUST-5 only fires on writes this system performs, so it never sees these.

  A redirection is the one exception: the harness opens that file itself, so what a line writes
  through `>` or `>>` is recorded
  ([tools/command-line.md](tools/command-line.md#CMDLINE-5)), under the name the line spelled and
  with the keying cost every other write has (below). A file the program opens on its own, which
  is `cmd -o notes.txt` or anything a build writes, is not.

  This is not an oversight and cannot be closed by watching the filesystem: by the time anything
  noticed, the question would be whether to distrust a file the user may have created themselves,
  and asking that on every change would make the map useless. What vouching for a directory means
  is a standing statement about that place, not about a set of files.

  The practical consequence is worth saying plainly: trusting a directory trusts what lands in it,
  so a tree that a build or a dependency manager writes into is a tree you are vouching for
  ahead of time.
- **Confinement is decided before an operation runs, not while it runs.** Where a path lands is
  worked out by resolving it, and the operation happens after that, so a component that is a
  directory when it is resolved and a symlink when the file is opened carries the bytes with it.
  Closing that needs every component opened in turn with link-following refused, which is a
  different walk from the one that answers where a path goes. A tree already arranged to escape is
  refused; one rearranged inside that window is not.
- **A rule is keyed on the name, so one file inside the workspace can have two.** Confinement
  resolves a path to where it lands, but the record is written and read under the name the
  operation used. A symlink inside the workspace therefore gives one file two spellings and two
  rules: content written as untrusted through one is read back as trusted under the other, which
  is the round trip TRUST-4 exists to close. Keying the record on the destination instead is what
  closes it, and that is a change to every rule the map holds rather than to confinement.
