---
id: PERM
title: Permission rules
status: normative
governs:
  - crates/core/src/permissions.rs
  - crates/config/src/settings.rs
  - crates/agent/src/permissions.rs
  - crates/agent/src/workspace.rs
  - crates/tui/src/trust_prompt.rs
  - crates/tui/src/app.rs
guards:
  - symbol: Policy::with_permissions
  - symbol: Policy::before_read
  - symbol: Policy::read_is_denied
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

Who answers a prompt a rule permitted to exist is [permission-modes.md](permission-modes.md). The
two are separate: a rule decides whether there is a question, and a mode answers one. A `deny` rule
therefore holds in every mode, including the mode that asks about nothing.

## What a rule is

<a id="PERM-1"></a>
### PERM-1: a rule names a family of tools, and matches on routing only

A rule is `Tool` or `Tool(specifier)`. Four families exist: `Read` covers every tool that reads or
enumerates a file, `Edit` covers every tool that changes one, `Bash` covers running a program, and
`WebFetch` covers fetching a URL. They are categories rather than tool names, and `Bash` names no
shell: there is none, and a specifier is matched against one stage's program and arguments.

A specifier is matched against a **routing** field and nothing else: a path, a stage's argv, or a
host. Never a file's contents, never a program's output, never anything else a turn observed.

`WebFetch` takes `domain:` and nothing else, so `WebFetch(domain:example.com)` covers that host and
its subdomains. A URL prefix would read as covering a path, and the boundary is a label boundary: a
rule about `example.com` never covers `notexample.com`. What a matching rule decides for a fetch,
and what it does not decide, is [fetch-url.md](tools/fetch-url.md).

**Why.** Routing is trusted and public before it reaches any gate, so matching on it is the driver
deciding from trusted input, which is what the driver is for. A rule matched against observed bytes
would be the driver branching on untrusted content, whatever the rule said.

`verified-by: bravebot_core::permissions::a_bare_family_name_covers_every_use_of_it`
`verified-by: bravebot_core::permissions::a_rule_for_one_family_does_not_decide_another`
`verified-by: bravebot_core::permissions::a_domain_rule_covers_the_host_and_its_subdomains`
`verified-by: bravebot_core::permissions::a_domain_rule_stops_at_a_label_boundary`
`verified-by: bravebot_core::permissions::a_domain_rule_ignores_case`
`verified-by: bravebot_core::permissions::a_bare_web_fetch_rule_covers_every_host`
`verified-by: bravebot_core::permissions::a_web_fetch_rule_decides_nothing_about_other_families`

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

An argument is never re-split, so a denied program cannot be smuggled inside one. The splitting is
done once, here, and nothing re-splits afterwards: a stage's argv is final by the time a rule is
matched against it, whether the stage arrived as an argument list or was compiled from a command
line. No shell ever sees either.

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

Nor is it about the directory a call happened to name, so it holds whichever way a walk arrives at
the file. A listing or a search consults the rules for every entry its walk reaches as well as for
the directory it was asked for, and an entry a rule covers is left out before the file is opened or
its name is reported. A directory a rule covers is not descended into. Both are decided ahead of
the walk's own caps, so a rule never costs a listing or a search the files it was asked about.

What comes back says a rule was the reason only where the rule is the whole reason: a search that
had nothing left to read reports that and that retrying is not the answer, and one that read the
rest of the tree reports what it found and nothing more. Saying which entries were left out, or how
many, would hand over the names the rule is keeping back, and a planner able to narrow a glob until
the notice appears has the names either way.

**Why.** A file whose contents are off limits is not protected if it can be overwritten, so the two
families are consulted together for a write. Enumerating a directory and searching it both report
what is in it, so a rule that fences a tree fences those too. A processor is the component allowed
to read what nobody vouched for, which makes it the route a rule most needs to cover rather than the
one it can afford to miss.

A deny rule also holds against a workspace the user vouched for, which is what makes one worth
writing: saying yes at startup trusts the whole tree, and a rule is how one file is kept out of that
answer without declining the rest of it. It holds against a mode that answers every prompt for the
same reason: the refusal comes before there is a prompt, so there is nothing for a mode to answer.

`verified-by: bravebot_core::policy::a_denied_program_does_not_run_at_all`
`verified-by: bravebot_agent::turn::a_denied_file_is_not_read_and_its_contents_do_not_reach_the_planner`
`verified-by: bravebot_agent::turn::a_denied_file_is_not_written_even_where_writes_are_approved`
`verified-by: bravebot_agent::turn::a_denied_file_is_not_read_by_a_processor_either`
`verified-by: bravebot_agent::turn::a_deny_rule_holds_against_a_trusted_workspace`
`verified-by: bravebot_agent::turn::a_deny_rule_holds_where_every_permission_check_is_bypassed`
`verified-by: bravebot_agent::turn::a_deny_rule_holds_when_a_search_walks_the_directory_above_the_file`
`verified-by: bravebot_agent::turn::a_deny_rule_holds_when_a_listing_walks_the_directory_above_the_file`
`verified-by: bravebot_agent::workspace::a_search_does_not_open_a_file_a_deny_rule_covers`
`verified-by: bravebot_agent::workspace::a_listing_does_not_enumerate_a_tree_a_deny_rule_covers`
`verified-by: bravebot_agent::workspace::a_denied_file_does_not_spend_a_searchs_budget`

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
`verified-by: bravebot_core::policy::private_input_asks_even_for_a_line_a_rule_allows`
`verified-by: bravebot_core::policy::a_reference_named_write_asks_whatever_a_rule_says`

<a id="PERM-10"></a>
### PERM-10: no rule extends reach, and `additionalDirectories` asks before it opens

An allow rule cannot make a path reachable that the workspace and the directories the user opened
do not already cover. A directory named in `additionalDirectories` is put to the person as a
question of its own when the session opens, and one they accept is opened by the same route
`/add-dir` takes and trusted for the session on the same terms. One they decline is neither
reachable nor vouched for. A relative name in it means a path under the workspace.

The mode that answers every permission question answers these too. A session resumed with the map
its own user left is asked nothing and opens none of them, since the directories it has open are
the ones its own record reopened. `/clear` closes the ones that were open and opens none, because
the answer that opened one was given by the session being cleared.

**Why.** Reach and asking are separate questions, and a rule about prompts must not answer the
other one by accident. These files are read before anything runs and the layers a checkout carries
arrive with the checkout, so a name in one that opened a directory by itself would be reach and
trust granted by whatever last edited the file: naming a directory asks for it, and a person grants
it. Sharing the route with `/add-dir` past that point is what keeps a directory a file named and a
directory a person typed from being reachable on different terms.

`verified-by: bravebot_agent::permissions::the_directories_a_file_named_come_back_in_order`
`verified-by: bravebot_tui::trust_prompt::a_directory_a_file_named_is_opened_only_where_the_person_accepts_it`
`verified-by: bravebot_tui::app::a_person_asked_about_the_workspace_is_asked_about_each_named_directory`
`verified-by: bravebot_tui::app::a_resume_that_brought_its_own_map_opens_no_directory_a_file_named`
`verified-by: bravebot_tui::app::an_accepted_directory_is_opened_and_trusted_like_one_typed`

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

## The question

<a id="PERM-13"></a>
### PERM-13: a question about a named directory names the directory it would open

A name in `additionalDirectories` is resolved before it is put to anybody, and the question shows
what it resolved to. A name that cannot be opened whatever the answer is reported instead of asked
about, and a directory two layers both named is one question.

**Why.** Opening a directory follows a name wherever it leads, so a question about the spelling
collects an answer about a different tree: a link inside a checkout resolves somewhere else
entirely, and the path in the box would begin with the person's own project while the grant landed
outside it. A question that changes nothing either way, and one already answered a box ago, both
train the habit of answering without reading, which is the whole of what asking is worth.

`verified-by: bravebot_tui::app::a_named_directory_is_resolved_before_it_is_asked_about`
`verified-by: bravebot_tui::app::a_directory_two_layers_both_named_is_asked_about_once`
`verified-by: bravebot_tui::app::a_name_that_cannot_be_opened_is_said_so_rather_than_asked_about`

## Known costs

- **How many questions a session opens with is the file's to choose.** Every name in
  `additionalDirectories` is one box, so a file naming thirty directories is thirty of them before
  the first prompt can be typed, and the way out of a list somebody does not want to answer is
  Ctrl-C, which starts no session. A cap would be worse: the names past it would be dropped in
  silence, which reads as a setting that does nothing. What limits the damage is that no box grants
  anything by itself.
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
  they would have got without it. The modes exist, and the command line and the mode key are what
  choose one: [permission-modes.md](permission-modes.md).
