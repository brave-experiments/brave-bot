---
id: TRUST
title: The trust map
status: normative
governs:
  - crates/core/src/trust.rs
  - crates/core/src/policy.rs
  - crates/tui/src/sessions.rs
  - crates/tui/src/dropped.rs
  - crates/agent/src/workspace.rs
guards:
  - symbol: TrustStore::trust
  - symbol: TrustStore::distrust
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
### TRUST-1: nothing is trusted until a rule says so

An empty map trusts no path, and a path's shape grants nothing. Every rule in the map was written
by one of the ways listed below, and each of those says what it grants and on what.

A session does not open empty. It opens with the two paths holding the user's standing instructions
for the project and nothing else, which is TRUST-13.

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
`verified-by: bravebot_core::trust::a_later_decision_replaces_an_earlier_one`

<a id="TRUST-3"></a>
### TRUST-3: relative and absolute rules are separate namespaces

Keeping two namespaces is a workaround rather than a preference, and
[issue #24](https://github.com/brave-experiments/brave-bot/issues/24) proposes replacing both with
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

`verified-by: bravebot_core::policy::trusted_data_into_a_trusted_path_is_silent_and_changes_nothing`
`verified-by: bravebot_core::policy::untrusted_data_into_a_trusted_path_prompts_and_distrusts_the_path`
`verified-by: bravebot_core::policy::trusted_data_into_an_untrusted_path_is_silent_and_trusts_the_path`
`verified-by: bravebot_core::policy::untrusted_data_into_an_untrusted_path_is_silent_and_changes_nothing`
`verified-by: bravebot_core::policy::an_unvouched_path_prompts_either_way`
`verified-by: bravebot_core::policy::a_file_written_with_untrusted_data_reads_back_untrusted`

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

A session begins with the opening rules and nothing an earlier session in that directory
accumulated. `/clear` begins a session and therefore begins again. `--resume` restores the map from
the record of the session chosen; a record from before maps were kept has none and opens as a fresh
one does.

**Why.** Every rule beyond the opening two is standing permission that something earned during a
session: a file a checker cleared, a path a person named, a directory they opened. Carrying those
into next week's session would grant them on behalf of a user who was there for none of it. A
resume is not an exception: the rules honoured are the ones that session accumulated, which is what
stops a resumed turn reading back a file an earlier turn of the same session poisoned.

`verified-by: bravebot_tui::sessions::a_record_that_predates_the_map_has_none_rather_than_an_empty_one`
`verified-by: bravebot_tui::sessions::a_distrusted_path_inside_a_trusted_tree_survives_the_record`
`verified-by: bravebot_tui::sessions::sessions_are_written_read_back_and_kept_per_directory`

<a id="TRUST-7"></a>
### TRUST-7: withdrawn, replaced by TRUST-13

There was a question at startup: whether the user trusted the working directory, with yes writing a
rule over the whole tree. It is gone, replaced by TRUST-13, and nothing asks in its place. A directory is somewhere to
work, not a statement about every file in it, and the question could only be answered by somebody
who had read none of them.

What each half of it became: the tree-wide content grant is gone entirely, and per-file content
trust is now TRUST-8's business; the two paths a session still opens trusting are TRUST-13.

`verified-by: by-construction (nothing asks at startup, and no code remains that could)`

## The ways a rule is written

Each grants exactly one thing. All but one grant it because a person made a gesture; the exception
is TRUST-8, which is the only rule in this map written on the strength of something having read the
content, and it says what that costs. TRUST-13 is the first, and is the only one written before
anything has happened. Two more write a rule the same way and have specs of their own: naming a
file is [naming-files.md](naming-files.md), and dropping one on the window is
[dropping.md](dropping.md).

<a id="TRUST-8"></a>
### TRUST-8: a read of a file nobody vouched for asks a checker, not a person

When a turn is about to be refused a file's contents, whether for a read or to locate the passage
an edit names, the whole file is offered to an isolated checker. A clean verdict writes exactly the rule `@` would have written, for that path and no
other, and the read proceeds as any read of a vouched-for path does. Anything else leaves the file
quarantined and the turn carries on with a reference. Recorded once per path, so a file is offered
at most once however many times it is read.

The whole file or nothing. A verdict about the first page of a file is a verdict about a document
nobody has, and a file too large to hand over in one piece gets no verdict.

A run may turn this off, and then a file nobody vouched for is simply quarantined.

**Why.** The question this replaces was a y/n in front of the person watching, once per path,
arriving while they waited for work to happen, about a file they already knew was in their own
project. It was answered yes almost every time, which is what a prompt looks like when it is a toll
rather than a decision. What it protected against is real, so it is still checked; what changed is
who does the checking.

The cost is that the grant is no longer a person's. Every other rule in this map comes from a
gesture no attacker can cause; this one is a model's opinion of bytes an attacker may have written,
so an attacker who owns a file gets to try. What that buys them is in
[vetting.md](vetting.md), which owns the checker itself.

`verified-by: bravebot_agent::turn::a_file_nobody_vouched_for_is_shown_once_a_checker_has_read_it`
`verified-by: bravebot_agent::turn::a_file_nobody_vouched_for_can_be_edited_once_a_checker_has_read_it`
`verified-by: bravebot_agent::turn::a_file_a_checker_will_not_clear_stays_quarantined`
`verified-by: bravebot_agent::turn::a_file_already_trusted_is_not_offered_to_a_checker`
`verified-by: bravebot_agent::turn::a_file_is_offered_to_a_checker_once_and_not_again_after_it_passes`
`verified-by: bravebot_agent::turn::vetting_can_be_turned_off_and_then_nothing_is_offered_to_a_checker`

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
whatever `/add-dir` has opened. `..` and an absolute path outside those are refused rather than
resolved, in an added directory exactly as in the project, and a symlink leaving one is refused.
A relative path always means the project, so no file has two spellings. Naming a directory
includes nothing, since a directory is somewhere to type through rather than a file to read.

`verified-by: bravebot_agent::workspace::an_absolute_path_outside_every_added_directory_is_still_refused`
`verified-by: bravebot_agent::workspace::a_parent_component_cannot_climb_out_of_an_added_directory`
`verified-by: bravebot_agent::workspace::a_symlink_out_of_an_added_directory_is_refused`

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

<a id="TRUST-13"></a>
### TRUST-13: a session opens by trusting the user's standing instructions, and nothing else

Two paths, written before any turn runs: `AGENTS.md` and `.bravebot/skills`. Written whether or not
either exists, since a rule is about a path rather than about what was there when it was made, and
a file written mid-session is read by the turn after it.

A **default, never an override.** A rule already covering one of those paths stays: a write that
poisoned the file and recorded it, or a person who said no, both knew more than a session start
does.

Not the working directory, whose files are TRUST-8's business. Not `.bravebot` whole, where a
repository may drop anything and only the skills directory is ever read as an instruction.

**Why.** These two are the files the user wrote in order to be obeyed. A session that treated them
as content nobody vouched for would ignore the very thing it was told to follow, and there is
nobody left to ask.

`verified-by: bravebot_agent::turn::a_workspace_agents_file_is_obeyed_without_anybody_vouching_for_it`
`verified-by: bravebot_agent::turn::an_untrusted_directory_is_not_reported_as_a_refusal`
`verified-by: bravebot_agent::turn::opening_a_session_does_not_undo_what_a_write_recorded`

## Known costs

Accepted deliberately. Do not "fix" one without changing this spec first.

- **A project's own `AGENTS.md` steers the agent from the first turn, whoever wrote it.** TRUST-13
  writes that rule before anything has read a byte of the file, so cloning a repository and
  starting a session in it puts that repository's standing instructions into the system prompt.
  Standing instructions are the highest-privilege content in the system: they are not data the
  planner weighs, they are the rules it works by, and nothing downstream re-examines them.

  This is the sharpest edge in the whole map and it is deliberate. The alternative was to put the
  file through the same checker every other file goes through, which would have caught the case
  and cost a call at the start of every session. What is accepted instead is that a repository you
  have not read is a repository you are taking instructions from.

  `bravebot_agent::turn::a_workspace_agents_file_is_obeyed_without_anybody_vouching_for_it` is
  written to fail if this ever stops being true, with the payload spelled out, so the exposure is
  visible in the suite rather than only here. Read it before changing this.

- **A fresh session forgets what an earlier one poisoned.** The rule that untrusted data marks
  its destination untrusted holds within a session and
  across a resume of it. Across a fresh start it cannot, because the map it was recorded in is
  gone, so a file one session marked untrusted is read as trusted by the next session that vouches
  for the directory. The alternative is a per-directory map, which is a directory that trusts
  itself. If a file holds content you do not trust, the answer is to say no to the directory, or
  to not leave it there.
- **A file another process rewrites after it was cleared stays cleared.** TRUST-2 makes a rule
  about the path rather than about the bytes that were there when it was written, so `npm install`,
  `git pull`, an editor, a background daemon, or a program the agent was allowed to run can all
  replace a file a checker has already passed, and it is read as trusted afterwards. TRUST-5 only
  fires on writes this system performs, so it never sees these.

  This is not an oversight and cannot be closed by watching the filesystem: by the time anything
  noticed, the question would be whether to distrust a file the user may have edited themselves,
  and asking that on every change would make the map useless. It is narrower than it was, since a
  clearance covers one file rather than a whole tree, but it is the same hole.
