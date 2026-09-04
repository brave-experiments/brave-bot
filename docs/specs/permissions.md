---
id: PERM
title: Permission rules
status: normative
governs:
  - crates/core/src/permissions.rs
  - crates/config/src/settings.rs
  - crates/agent/src/permissions.rs
guards:
  - symbol: Policy::with_permissions
  - symbol: Policy::before_read
  - symbol: Policy::before_write
  - symbol: Policy::before_run_rules
---

## Scope

Rules a person writes down in advance, in the `permissions` block of `~/.bravebot/settings.json`,
about which actions to ask them about and which to refuse outright. The same three lists Claude
Code keeps, with the same spellings, so a block copied out of `~/.claude/settings.json` governs
this agent unedited.

What a rule may decide is narrow, and the boundary is the point. A rule decides **whether a person
is asked** and **whether an action happens at all**. It never decides what a value is trusted for,
which is [labels.md](labels.md), and never what is reachable, which is
[trust-map.md](trust-map.md). `defaultMode` is read and acted on by nothing: this spec covers the
three lists and `additionalDirectories`.

## What a rule is

<a id="PERM-1"></a>
### PERM-1: a rule names a family of tools, and matches on routing only

A rule is `Tool` or `Tool(specifier)`. Three families exist: `Read` covers every tool that reads or
enumerates a file, `Edit` covers every tool that changes one, and `Bash` covers running a program.
They are categories rather than tool names, and `Bash` names no shell: there is none, and a
specifier is matched against one stage's program and arguments.

A specifier is matched against a **routing** field and nothing else: a path, or a stage's argv.
Never a file's contents, never a program's output, never anything else a turn observed.

**Why.** Routing is trusted and public before it reaches any gate, so matching on it is the driver
deciding from trusted input, which is what the driver is for. A rule matched against observed bytes
would be the driver branching on untrusted content, whatever the rule said.

`verified-by: bravebot_core::permissions::a_bare_family_name_covers_every_use_of_it`
`verified-by: bravebot_core::permissions::a_rule_for_one_family_does_not_decide_another`

<a id="PERM-2"></a>
### PERM-2: deny, then ask, then allow, and the first match decides

Specificity does not enter into it. A broad deny beats a narrow allow, so a deny rule cannot carry
exceptions, and a matching ask rule prompts even where a more specific allow rule also matches.

**Why.** It is what makes a deny rule readable as a flat statement about what will not happen. A
list where the narrowest rule won could not be checked by reading it.

`verified-by: bravebot_core::permissions::deny_beats_ask_and_ask_beats_allow_however_specific_the_loser`

<a id="PERM-3"></a>
### PERM-3: a path specifier is gitignore-shaped, and says where it starts

`*` matches within one segment and `**` across them. A trailing `/**` covers the directory it names
as well as what is under it. Four anchors decide where a pattern begins:

| Written | Starts at |
|---|---|
| `//x` | the filesystem root |
| `~/x` | the user's home directory |
| `/x` | the directory the settings file is in |
| `x` or `./x` | the workspace |

A single leading slash is therefore **not** the filesystem root. A specifier with no slash in it is
a name and matches at any depth, so `Read(.env)` and `Read(**/.env)` are one rule. Relative and
absolute patterns are separate namespaces and neither reaches into the other, the same separation
the trust map keeps. A pattern whose anchor is unknown, such as `~/` on a machine with no home
directory, is reported as unusable rather than silently matching nothing.

`verified-by: bravebot_core::permissions::a_bare_name_matches_at_any_depth_in_every_list`
`verified-by: bravebot_core::permissions::each_anchor_points_where_its_leader_says`
`verified-by: bravebot_core::permissions::one_star_stays_in_a_segment_and_two_cross_them`
`verified-by: bravebot_core::permissions::a_trailing_double_star_covers_the_directory_it_names`
`verified-by: bravebot_core::permissions::a_relative_rule_says_nothing_about_an_absolute_path`
`verified-by: bravebot_core::permissions::an_anchored_pattern_matches_only_where_it_is_anchored`
`verified-by: bravebot_core::permissions::a_pattern_whose_anchor_is_unknown_is_reported_rather_than_matching_nothing`
`verified-by: bravebot_agent::permissions::a_single_slash_rule_is_anchored_at_the_settings_directory`

<a id="PERM-4"></a>
### PERM-4: a one-segment relative pattern floats where it restricts, not where it grants

`Edit(src/**)` in `deny` or `ask` covers a `src` directory at any depth, including a nested copy
under `vendor`. The same pattern in `allow` covers only the `src` at the top.

**Why.** A rule that restricts should cover the copy somebody forgot about; a rule that grants
should cover what it says and no more. Anchoring the pattern, as `Edit(/src/**)`, pins it to one
place in every list.

`verified-by: bravebot_core::permissions::a_single_segment_directory_floats_when_it_restricts_and_not_when_it_grants`

<a id="PERM-5"></a>
### PERM-5: a command specifier matches the whole line, with `*` standing in for any text

A rule with no `*` matches one exact command. A trailing ` *` also matches the bare command, but
only when it is the rule's only wildcard, so `Bash(ls *)` covers `ls` and `Bash(* --help *)` does
not cover `npm --help`. The space before a trailing `*` is part of the rule: `Bash(ls *)` does not
match `lsof` and `Bash(ls*)` does. A trailing `:*` is the same rule as a trailing ` *`, and a colon
anywhere else is an ordinary character.

`verified-by: bravebot_core::permissions::a_command_pattern_matches_where_the_documented_table_says`
`verified-by: bravebot_core::permissions::a_trailing_colon_star_is_a_trailing_wildcard_and_a_colon_elsewhere_is_not`

<a id="PERM-6"></a>
### PERM-6: every stage of a pipeline is judged on its own

Restricting any one stage restricts the pipeline. Granting it needs every stage granted: one stage
no rule covers is a program nobody has answered for, and what it prints is what the next stage
reads.

An argument is never re-split, so a denied program cannot be smuggled inside one. There is no shell
to do the splitting, which is what makes this hold rather than a matter of parsing carefully.

`verified-by: bravebot_core::permissions::a_pipeline_is_allowed_only_when_every_stage_is`
`verified-by: bravebot_core::permissions::restricting_one_stage_restricts_the_whole_pipeline`
`verified-by: bravebot_core::policy::a_denied_stage_cannot_hide_between_two_permitted_ones`
`verified-by: bravebot_core::policy::a_denied_program_cannot_be_smuggled_inside_an_argument`

## What a rule does

<a id="PERM-7"></a>
### PERM-7: a deny rule refuses before anything is opened or started

A denied file is not read, not enumerated, and not written, and a denied program does not run: the
refusal comes before the file is opened or the program is looked for. A `Read` deny rule also stops
a write to the path it covers.

The rule is about the file, not about the spelling used to ask for it. Naming a path through a
reference reaches the same refusal, including on the one route that may read quarantined content: a
processor is handed no denied file either. The planner is told the rule refused and that retrying is
not the answer, and where the path arrived through a reference the refusal names the reference rather
than the path, as everything else that goes back to the planner does.

**Why.** A file whose contents are off limits is not protected if it can be overwritten, so the two
families are consulted together for a write. Enumerating a directory and searching it both report
what is in it, so a rule that fences a tree fences those too. A processor is the component allowed
to read what nobody vouched for, which makes it the route a rule most needs to cover rather than the
one it can afford to miss.

A deny rule also holds against a workspace the user vouched for, which is what makes one worth
writing: saying yes at startup trusts the whole tree, and a rule is how one file is kept out of that
answer without declining the rest of it.

`verified-by: bravebot_core::policy::a_denied_program_does_not_run_at_all`
`verified-by: bravebot_agent::turn::a_denied_file_is_not_read_and_its_contents_do_not_reach_the_planner`
`verified-by: bravebot_agent::turn::a_denied_file_is_not_written_even_where_writes_are_approved`
`verified-by: bravebot_agent::turn::a_denied_file_is_not_read_by_a_processor_either`
`verified-by: bravebot_agent::turn::a_deny_rule_holds_against_a_trusted_workspace`

<a id="PERM-8"></a>
### PERM-8: an allow rule answers a prompt and grants nothing else

It stops the asking. It does **not** make a program's output trusted, and it does not raise any
label: output carries what it would have carried, which is untrusted unless a person vouched for
every stage.

**Why.** Vouching at a prompt grants those two things together because a person is looking at one
command and can answer for both. A pattern covers commands nobody has read, so it cannot carry the
second claim. If a rule could trust output, one line in a settings file would turn fetched bytes
into routing, which is the whole thing labels exist to prevent.

`verified-by: bravebot_core::policy::a_rule_the_user_wrote_in_advance_answers_the_run_prompt`
`verified-by: bravebot_core::policy::an_allow_rule_stops_the_prompt_and_does_not_trust_what_the_command_prints`
`verified-by: bravebot_agent::turn::an_allow_rule_reaches_the_path_it_names_and_no_other`

<a id="PERM-9"></a>
### PERM-9: two prompts no rule can answer

A run that would put the user's private data into a program asks whatever the rules say. A write
whose destination is known only through a reference asks whatever the rules say.

**Why.** They are not the same question a rule answers. The first is about confidentiality: a rule
saying which commands may run is not consent to hand one the user's data, exactly as vouching for a
command is not. The second is structural: that prompt is the only moment such a path is shown to
anybody, and the endorsement is minted for the path the person saw, so nothing a pattern says can
stand in for having looked.

`verified-by: bravebot_core::policy::private_input_asks_even_for_a_command_a_rule_allows`
`verified-by: bravebot_core::policy::a_reference_named_write_asks_whatever_a_rule_says`

<a id="PERM-10"></a>
### PERM-10: no rule extends reach, and `additionalDirectories` grants no more than `/add-dir`

An allow rule cannot make a path reachable that the workspace and the directories the user opened
do not already cover. A directory named in `additionalDirectories` is opened by the same route
`/add-dir` takes and is trusted for the session on the same terms, and a relative name in it means
a path under the workspace.

**Why.** Reach and asking are separate questions, and a rule about prompts must not answer the
other one by accident. Sharing one route with `/add-dir` is what keeps a directory a file named
and a directory a person typed from being reachable on different terms.

`verified-by: bravebot_agent::permissions::the_directories_a_file_named_come_back_in_order`
`verified-by: bravebot_tui::app::a_settings_file_directory_is_opened_and_trusted_like_one_typed`

## The file

<a id="PERM-11"></a>
### PERM-11: an unreadable rule is dropped, named, and takes nothing with it

A line that is not a rule, names no family this agent has, or has no anchor to resolve is dropped,
and the rest of the file still applies. Every one dropped is reported: on `doctor`, and in the
session where the file was read.

**Why.** A misspelled deny rule reads as protection that is not there, which is the one failure
here worth interrupting somebody over. Refusing the whole file instead would mean a typo in an
allow rule quietly removed a deny rule's protection.

`verified-by: bravebot_core::permissions::a_rule_that_cannot_be_read_is_dropped_and_reported`
`verified-by: bravebot_core::permissions::one_unreadable_rule_does_not_discard_the_others`
`verified-by: bravebot_config::settings::an_entry_that_is_not_a_rule_is_left_out`
`verified-by: bravebot_config::settings::a_malformed_permissions_block_carries_no_rules`
`verified-by: bravebot_agent::permissions::a_line_that_is_not_a_rule_is_reported`

<a id="PERM-12"></a>
### PERM-12: no rules means no change

A session with no `permissions` block behaves exactly as one did before the block existed: every
gate asks what it asked before, and nothing is refused for being unmentioned. The rules are read
once per session, so a file edited while a session is open describes the next one.

**Why.** The lists are empty by default and the empty case is the common one. A feature that
altered a session nobody had configured would be a change to every session.

`verified-by: bravebot_core::permissions::no_rules_decide_nothing`
`verified-by: bravebot_agent::permissions::no_block_is_no_rules`
`verified-by: bravebot_config::settings::the_permissions_block_and_the_env_block_do_not_need_each_other`

## Known costs

- **A rule is matched against argv, not against what a program does.** `Bash(git *)` covers
  `git -c core.fsmonitor=<script> diff`, which runs a program the rule never named, and
  `Bash(devbox run *)` covers whatever follows `run`. A pattern constraining arguments is weaker
  than it looks, and a `deny` list is not a sandbox: [sandboxing.md](sandboxing.md) is what
  confines a process, and the label on a program's output is what holds regardless.
- **A path rule does not reach a program's own file access.** `Read` and `Edit` rules govern the
  tools that read and write files. A program is argv to this agent, so `run cat .env` is checked
  against the `Bash` rules and against the run prompt, not against a `Read` rule covering `.env`.
  Every run asks unless a person vouched for that exact command, so the prompt is what stands
  there; a `Bash` deny rule, or the sandbox, is what closes it. Naming a path in `deny` and
  expecting it to fence every subprocess would be believing something that is not true.
- **`defaultMode` is read and does nothing.** The key is parsed so the file is not rejected for
  carrying it, and no mode is selected from it. A person who wrote `acceptEdits` gets the prompts
  they would have got without it.
