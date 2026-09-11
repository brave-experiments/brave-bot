---
id: CMDLINE
title: command lines
status: proposed
governs:
  - crates/agent/src/cmdline.rs
  - crates/agent/src/exec.rs
  - crates/agent/src/tools.rs
  - crates/agent/src/programs.rs
  - crates/core/src/command.rs
  - crates/core/src/pure.rs
  - crates/core/src/permissions.rs
  - crates/core/src/policy.rs
  - crates/agent/src/confirm.rs
  - crates/tui/src/confirm.rs
guards:
  - symbol: cmdline::compile
  - symbol: Policy::before_run
  - symbol: Policy::read_output
---

## Scope

Letting the planner express a command the way a person would write one, and what that costs.
Replaces the argv-array surface of [run.md](run.md) as the planner's spelling, keeps every effect
that spelling had, and adds the three things its absence made impossible: filtering at the source,
reading a result without a second round trip, and running something long without holding the turn.

Shell mode, the `!` prompt, is [shell-mode.md](../shell-mode.md) and is untouched. It runs a line
through a real shell because a person typed it. Nothing here does that.

## The parity target

What the equivalent tool in Claude Code offers, as the list this spec is measured against:

| | Claude Code | `run` today | This spec |
|---|---|---|---|
| Command spelling | one shell string | array of argv stages | one command line, compiled |
| `\|`, `&&`, `\|\|`, `;` | yes | pipe only | yes |
| Redirection | yes | no | yes, as a write destination |
| Globs | yes, by the shell | no | yes, by the harness |
| `$(...)`, backticks | yes | no | **refused** |
| `$VAR` | yes | no | **refused** |
| Output to the model | direct text | reference, then a second call | direct where the label allows |
| Truncation | head and tail kept | none | head and tail kept |
| Deadline | per call, 2min default, 10min ceiling | fixed 300s | per call, ceiling |
| Background | yes, wakes on exit | no | yes, wakes on exit |
| Working directory | persists between calls | per call | persists between calls |
| Shell state (`export`, functions) | does not persist | n/a | does not persist |
| Permission rules | per sub-command prefix | per stage | per compiled stage |

The two rows in bold are where parity is refused on purpose, and [CMDLINE-2](#CMDLINE-2) says why.
Everything else is adopted.

## Why the shell exclusion said never, and what changes

[shell-mode.md](../shell-mode.md) ends with a clause saying the planner gets no shell tool, ever,
"not behind a capability, not behind an approval prompt". [run.md](run.md) gives the reasoning:
a shell string is destination and payload at once, so there is nothing in it a person could
approve on its own, and a parser that tried to work out what it means would be racing a shell it
does not control.

Both halves of that are about **handing a string to an interpreter**. Neither is about the
syntax. The move here is to keep the ban on the interpreter and drop the ban on the notation:

> The planner does not get a shell. It gets shell **syntax**, which the harness compiles into the
> same argv stages `run` executes today, and refuses to compile when it cannot say exactly what
> would happen.

Nothing is ever passed to `sh -c`. The compiled plan is what runs, spawned by resolved path with
an argument vector, through the same process plumbing as today. So:

- **The routing field exists again.** It is the compiled plan: a list of stages with resolved
  binaries and literal argv, plus every file the plan would write. That is a thing a person can
  read and endorse, which is the property [run.md](run.md) protects by having a person endorse
  argv, and the property a raw string lacked.
- **The parser is not racing anything.** It is not predicting what a shell will do with a string,
  because no shell will ever see the string. It is the only thing that reads it, and its output is
  the whole of what happens.
- **Ambiguity is refused, never guessed.** A construct the compiler cannot fully resolve is an
  error, and there is no path from that error to running the line anyway.

The distinction to hold onto is the same one `run.md` already draws, moved one step: it is not
that shell strings turned out to be acceptable, it is that the exclusion was about *interpretation*
and a compiled plan is not interpreted.

## Clauses

<a id="CMDLINE-1"></a>
### CMDLINE-1: the planner sends a command line, and it is compiled rather than interpreted

```
run { command: "grep -rn kBraveWalletDisabledByPolicy components/ | head -30" }
```

The line is parsed by this repository's own grammar into an ordered plan of stages, each a
resolved absolute path and a literal argument vector, together with the plan's write set, its read
set, and its control flow. That plan is what executes, through the existing argv spawner. No shell
is started at any point, and no part of the line is ever concatenated back into a string that
something else parses.

An argument that survives compilation is final. `; rm -rf /` inside quotes is one argument and
arrives as one argument, because the only thing that ever split the line was the compiler, and it
already ran.

`verified-by: bravebot_agent::tools::run_takes_one_command_line_and_nothing_else`
`verified-by: bravebot_agent::turn::a_line_with_a_pipe_is_compiled_into_its_steps`
`verified-by: bravebot_agent::exec::a_command_line_runs_as_the_plan_it_compiled_to`
`verified-by: bravebot_agent::exec::a_command_line_chains_its_steps`
`verified-by: bravebot_agent::exec::a_metacharacter_inside_quotes_reaches_the_program_as_one_argument`
`verified-by: bravebot_agent::exec::a_redirection_inside_quotes_writes_no_file`
`verified-by: bravebot_agent::cmdline::a_step_carries_the_file_its_name_resolved_to`

<a id="CMDLINE-2"></a>
### CMDLINE-2: the grammar is closed, and a refusal is never a fallback

Accepted, and nothing else:

| Construct | Form |
|---|---|
| Simple command | `word...` |
| Pipeline | `cmd \| cmd \| …` |
| Sequencing | `&&`, `\|\|`, `;` |
| Grouping | `( … )` for sequencing only, no subshell semantics |
| Quoting | `'…'`, `"…"`, `\` |
| Redirection | `>`, `>>`, `<`, `2>`, `2>>`, `2>&1`, `&>` with a literal target |
| Globs | `*`, `?`, `[…]`, `**` in operand position |
| Brace expansion | `{a,b}`, `{1..9}` |
| Home | leading `~` |
| Per-command environment | `NAME=literal cmd` |

Refused, as a compile error with the offending span named:

| Construct | Why |
|---|---|
| `$(…)`, `` `…` `` | a command whose text is computed is a destination nobody saw |
| `<(…)`, `>(…)` | same, plus a file descriptor nobody named |
| `$VAR`, `${…}` | the value is not in the line, so the plan is not in the line |
| `$((…))` | arithmetic is a language, and a language needs an interpreter |
| `&` | backgrounding is a parameter of the call, not a token in the line |
| `<<`, `<<<` | here-documents are content wearing the shape of syntax |
| `eval`, `source`, `.`, `exec`, `trap` | reintroduce interpretation by name |
| `if`, `while`, `for`, `case`, `function` | control flow is a program |
| `!` | history expansion is text the user typed reaching a line the planner wrote |

**A refusal returns an error and runs nothing.** There is no degraded mode, no "fall back to
`sh -c`", no "run the prefix that did compile". A compiler that can be made to give up and hand the
string to a shell is a shell with extra steps, and it is the single change that would void every
clause here.

**Why `$VAR` is refused when a shell would allow it.** The plan a person endorses has to be the plan
that runs. `rm -rf "$D/build"` is endorsable only if `D` is known at the prompt, and if it is known
then the planner could have written it. Substituting it before the prompt makes the feature
pointless; substituting it after makes the endorsement a lie. A closed set of harness-supplied
names (the workspace root, say) could be admitted later by expanding them at compile time, which
is the same thing as the planner having written them.

`verified-by: bravebot_agent::cmdline::a_simple_command_is_its_program_and_its_operands`
`verified-by: bravebot_agent::cmdline::a_pipeline_keeps_its_stages_in_order`
`verified-by: bravebot_agent::cmdline::and_binds_tighter_than_a_semicolon`
`verified-by: bravebot_agent::cmdline::parentheses_group_a_sequence`
`verified-by: bravebot_agent::cmdline::a_backslash_makes_the_next_character_ordinary`
`verified-by: bravebot_agent::cmdline::each_redirection_form_is_read_as_the_stream_it_names`
`verified-by: bravebot_agent::cmdline::a_glob_survives_as_a_pattern_and_a_quoted_one_does_not`
`verified-by: bravebot_agent::cmdline::braces_are_alternatives_or_a_range`
`verified-by: bravebot_agent::cmdline::a_leading_tilde_stands_for_a_home_and_a_later_one_does_not`
`verified-by: bravebot_agent::cmdline::an_assignment_in_front_of_the_program_is_environment`
`verified-by: bravebot_agent::cmdline::a_command_whose_text_would_be_computed_is_refused`
`verified-by: bravebot_agent::cmdline::process_substitution_is_refused`
`verified-by: bravebot_agent::cmdline::a_variable_is_refused`
`verified-by: bravebot_agent::cmdline::arithmetic_is_refused`
`verified-by: bravebot_agent::cmdline::backgrounding_is_refused`
`verified-by: bravebot_agent::cmdline::a_here_document_is_refused`
`verified-by: bravebot_agent::cmdline::a_program_that_reintroduces_interpretation_is_refused`
`verified-by: bravebot_agent::cmdline::a_word_that_opens_control_flow_is_refused`
`verified-by: bravebot_agent::cmdline::an_unquoted_exclamation_mark_is_refused`
`verified-by: bravebot_agent::cmdline::quoting_makes_a_refused_construct_ordinary_text`
`verified-by: bravebot_agent::cmdline::a_line_that_half_compiles_yields_nothing`

<a id="CMDLINE-3"></a>
### CMDLINE-3: what a person endorses is the compiled plan, not the line they were sent

The prompt shows the plan: each stage's resolved binary and argv with argument boundaries visible,
each file the plan may write, and the directory it runs in. The line as the planner spelled it is
shown as well, above the plan, as context, but it is not what the endorsement binds to.

An endorsement binds to the plan. Two different lines that compile to the same plan are the same
endorsement; one line that compiles to two different plans on two occasions, which glob expansion
makes possible, is two endorsements.

**Why.** The routing field is the thing that decides where an effect lands, and after compilation
that is the plan. Binding to the text would let a change in the working tree change what a prior
approval covers without the text changing at all.

`verified-by: bravebot_tui::confirm::a_run_prompt_shows_the_plan_it_would_endorse`
`verified-by: bravebot_tui::confirm::a_run_prompt_shows_the_line_the_model_wrote_as_context`
`verified-by: bravebot_tui::confirm::a_run_prompt_lists_every_file_the_line_would_write`
`verified-by: bravebot_core::command::a_plan_shows_its_resolved_binaries_and_argument_boundaries`
`verified-by: bravebot_core::command::two_plans_never_encode_alike`
`verified-by: bravebot_core::command::the_line_a_plan_came_from_is_not_part_of_what_it_encodes`
`verified-by: bravebot_core::policy::an_endorsement_does_not_authorise_a_differently_joined_plan`
`verified-by: bravebot_core::policy::an_endorsement_does_not_authorise_a_plan_that_writes_elsewhere`

<a id="CMDLINE-4"></a>
### CMDLINE-4: globs are expanded by the harness, before the prompt, and bounded

Expansion happens against the workspace at compile time, so the person sees the file list rather
than the pattern. A pattern matching nothing is a compile error and not an argument passed through
literally, which is what a shell would do and is never what anyone meant.

Expansion is bounded. Past the bound the compile fails and says the count, rather than putting a
thousand paths in front of a reader who will approve them unread.

Ignored directories are ignored the way the tree walk ignores them, so `**` does not descend into
`.git` or `node_modules`.

**Why bounded.** An approval prompt long enough that nobody reads it is a prompt that grants
everything and asks nothing.

`verified-by: bravebot_agent::cmdline::a_pattern_becomes_the_files_it_matches`
`verified-by: bravebot_agent::cmdline::a_pattern_matches_within_one_segment_and_a_tree_across_them`
`verified-by: bravebot_agent::cmdline::a_pattern_matching_nothing_is_refused_rather_than_passed_through`
`verified-by: bravebot_agent::cmdline::expansion_is_bounded_and_the_refusal_says_the_count`
`verified-by: bravebot_agent::cmdline::a_tree_pattern_does_not_descend_into_an_ignored_directory`
`verified-by: bravebot_agent::cmdline::a_dot_file_is_matched_only_by_a_pattern_that_writes_the_dot`
`verified-by: bravebot_agent::cmdline::a_class_matches_the_characters_it_names`
`verified-by: bravebot_agent::cmdline::a_word_with_no_pattern_names_a_file_that_need_not_exist`
`verified-by: bravebot_agent::cmdline::a_quoted_pattern_is_not_expanded`
`verified-by: bravebot_agent::cmdline::braces_multiply_a_word`
`verified-by: bravebot_agent::cmdline::a_range_counts_and_keeps_the_padding_it_was_written_with`
`verified-by: bravebot_agent::cmdline::a_leading_tilde_becomes_the_home_directory`
`verified-by: bravebot_agent::cmdline::a_tilde_with_no_home_to_stand_for_is_refused`

<a id="CMDLINE-5"></a>
### CMDLINE-5: a redirection target is a write destination and takes the write gates

`> out.txt` is a file this plan creates or truncates. It is routing, it appears in the plan's write
set, it is shown at the prompt, and it is subject to every rule that governs writing a file: the
permission rules, the trust map's answer for that path, and the confinement that keeps a write
inside the workspace.

`>>` is a write. `<` is a read and joins the read set, and it is standard input as well: the
file's bytes go into a program, so the plan reports it as private input and it takes the
standard-input gate of [run.md](run.md#RUN-6) whichever file it names. `2>&1` renames a
descriptor and touches no file.

A redirection whose target is a glob, or anything else that does not compile to one literal path,
is a compile error.

**Why.** This is the construct that most obviously fuses destination with payload in a raw string,
and it is exactly the one that becomes safe once the destination is a named field. Refusing
redirection while allowing pipes would be refusing the easy half.

`verified-by: bravebot_agent::cmdline::a_redirection_joins_the_write_set`
`verified-by: bravebot_agent::cmdline::an_append_writes_and_an_input_reads`
`verified-by: bravebot_agent::cmdline::an_input_redirection_is_private_input`
`verified-by: bravebot_agent::cmdline::a_line_that_only_writes_releases_nothing`
`verified-by: bravebot_agent::cmdline::joining_the_streams_writes_no_file`

<a id="CMDLINE-6"></a>
### CMDLINE-6: every branch that could run is endorsed before anything runs

`a && b` may run `b`. `a || b` may run `b`. `a ; b` runs `b`. All of them are in the plan, all of
them are shown, and all of them are endorsed up front. Nothing is approved lazily part-way through
a line, because a person answering a second prompt in the middle of a running line cannot tell what
state the first half left behind.

A stage that does not run because a branch was not taken is not an effect and needs no separate
answer. It was still endorsed, and that is the conservative direction.

`verified-by: bravebot_agent::cmdline::every_branch_that_could_run_is_in_the_plan`
`verified-by: bravebot_core::command::a_plan_lists_every_step_that_could_run`
`verified-by: bravebot_core::policy::an_endorsement_does_not_authorise_a_differently_joined_plan`
`verified-by: bravebot_agent::exec::the_right_side_of_and_runs_only_when_the_left_succeeded`
`verified-by: bravebot_agent::exec::the_right_side_of_or_runs_only_when_the_left_failed`
`verified-by: bravebot_agent::exec::a_semicolon_runs_both_sides_whatever_the_first_did`
`verified-by: bravebot_agent::exec::a_group_sequences_the_steps_it_holds`

<a id="CMDLINE-7"></a>
### CMDLINE-7: permission rules match compiled stages, one at a time

A rule's command specifier is matched against each stage of the compiled plan, rendered as its
program name and argv joined by single spaces. Restricting any one stage restricts the whole line.
Granting the line needs every stage granted.

This is the rule that already governs pipelines, applied to a longer plan. What changes is that the
splitting is now done by the compiler rather than being impossible; what does not change is that an
argument is never re-split, so a denied program cannot be smuggled inside one.

A rule may also name a write destination, and a redirection target is matched as a path the same way
`write_file`'s destination is.

`verified-by: bravebot_core::policy::a_denied_step_refuses_the_whole_line`
`verified-by: bravebot_core::policy::a_rule_denying_a_path_denies_a_redirection_to_it`
`verified-by: bravebot_core::policy::a_rule_denying_a_path_denies_reading_it_into_a_line`
`verified-by: bravebot_core::policy::a_denied_program_cannot_be_smuggled_inside_an_argument`
`verified-by: bravebot_core::permissions::a_pipeline_is_allowed_only_when_every_stage_is`
`verified-by: bravebot_core::permissions::restricting_one_stage_restricts_the_whole_pipeline`

<a id="CMDLINE-8"></a>
### CMDLINE-8: a plan may prove its output's label from what it read

The audited table that today proves a stdin-only filter's output is a function of its input is
extended to plans that name files. A stage is **read-proven** when its resolved program is in the
table and, for the exact argv given, the table's audit establishes all four:

1. it writes no file;
2. it executes nothing and starts no process;
3. it opens no socket;
4. every byte it reads comes from stdin or from a path in its operands.

The plan's read set is those operand paths, glob-expanded. The output's first label is the meet of
the labels over the read set and over stdin, which is the ordinary rule for a derived value,
applied to a process, and is not a relabel and grants nothing.

Where every stage is read-proven and every path in the read set is one the trust map answers for,
the output is trusted and the planner reads it. Where any stage is not, or any path is outside what
the trust map covers, the output is untrusted and private exactly as it is today.

**This is the proof road, and it is not the assertion road.** A person vouching for a command is a
person taking responsibility for it. This is a claim about a program, checked against that program's
option surface by hand, and nothing a user says extends the table.

**`grep -r` becomes provable, and that is the point.** Recursion is denied in the table today
precisely because the output would be labelled from stdin while the data came from disk. With a read
set that names the directory, the label comes from the directory, and the single most useful
exploration command in a large repository stops being opaque.

**`git` is not provable and does not go in the table.** A repository's own config can define an
alias that runs a command and a pager that runs a command, so `git log` is an interpreter whose
program is a file in the tree being inspected. It stays on the assertion road, where a person
vouches for it. Recognising a safe git invocation means reading `.git/config`, which is content, to
decide routing, and that is the thing that is never allowed.

`sed` and `awk` never enter the table, for the reason they are excluded today: they are
interpreters, and `awk`'s `system()` reaches the shell this repository excludes.

`verified-by: none`

<a id="CMDLINE-9"></a>
### CMDLINE-9: a read-proven plan does not prompt

Where every stage of a plan is read-proven under [CMDLINE-8](#CMDLINE-8), the plan's write set is
empty, and its read set lies inside what the trust map answers for, the line runs without asking.

This is a narrowing of the rule that every run asks. That rule's reason is that `foo --bar` might
write to disk and nothing here can tell. For an entry in the audited table, with an audited argv,
something here **can** tell, and the table is the record of having checked. The proof is the only
thing that may answer the prompt: never a declaration by a stage, never a property inferred from a
program's name, never anything derived from what a program printed.

A permission rule that says `ask` or `deny` for a stage still decides. Proof removes the default
prompt; it does not overrule a rule a person wrote.

**Why this clause is the risky one.** It is the only place in this spec where a proof stops a human
from being asked, and if the table is wrong somewhere then something ran that nobody saw. The
mitigations are that the table is small, hand-audited per program against its full option list,
keyed by resolved path rather than by name, and fails closed on any argv it does not fully
recognise. If a reviewer will not accept it, [CMDLINE-8](#CMDLINE-8) still stands on its own and
still removes the second round trip; what is lost is the first prompt, not the readable output.

`verified-by: none`

<a id="CMDLINE-10"></a>
### CMDLINE-10: output comes back as text whenever the planner may read it

A result the planner is allowed to read is returned in the tool result, in full, up to the bound in
[CMDLINE-11](#CMDLINE-11). Not as a reference to be fetched by a second call.

That covers output that is trusted by proof under [CMDLINE-8](#CMDLINE-8) and output of a command a
person vouched for, which is already trusted today and is already returned as text. What changes is
that the trusted case stops being rare.

Output the planner may not read is quarantined as a reference, and asking to see it is unchanged.

Standard error comes back with standard output, separated and labelled, under the same rule. A run
that failed put its explanation on stderr, and a planner that cannot see it reports that the command
worked.

**Why this matters more than it looks.** A reference plus a second call is one extra model round
trip per command. In the session that motivated this spec, a round trip cost between five and a
hundred and fifty seconds. A tool that costs two of those to answer `which files mention this
symbol` is a tool nobody can afford to use, which is why the harness's own guidance had steered the
planner away from it.

`verified-by: none`

<a id="CMDLINE-11"></a>
### CMDLINE-11: output is bounded, and what was dropped is said

A result returned as text is capped. Past the cap the head and the tail are kept and the middle is
dropped, with a line in between naming how many bytes and lines went. Head and tail rather than head
alone, because a build log's verdict is at the end and its first error is near the beginning.

The cap is on what enters the conversation, not on what the command printed. The whole output stays
available as a reference, so a planner that needs the middle can hand it to a processor or write it
to a file without running the command again.

The cap is a context-budget decision and belongs beside the other ones: a single tool result must
never be able to spend a large fraction of the conversation.

`verified-by: none`

<a id="CMDLINE-12"></a>
### CMDLINE-12: the working directory persists across calls, and shell state does not

A call may name a directory to run in, inside the workspace or inside a directory the user added.
Absent one, a line runs where the last one ran, and the first runs at the workspace root.

Nothing else carries over. `NAME=value` on one line does not affect the next, because there is no
shell process between calls to hold it: each stage is spawned fresh with the process environment,
less this agent's own credentials. A planner that needs a variable set puts it on the line that
needs it.

**Why the asymmetry.** A directory is a routing field, shown at every prompt and endorsed with the
plan, so carrying it is visible. An environment that accumulated invisibly across calls would change
what a later plan does without appearing in that plan.

`verified-by: none`

<a id="CMDLINE-13"></a>
### CMDLINE-13: a call may name its own deadline, under a ceiling

The default is short enough that a hung program is noticed and long enough for an ordinary build
step. A call may raise it up to a ceiling it cannot exceed.

Reaching the deadline ends the run rather than failing it, exactly as the fixed limit does today:
the stages are killed, what they printed is collected and returned under whatever label it had
earned, and the stop is reported as structure so a caller can tell the two apart without reading a
byte. Collection after a kill stays abandonable, because a killed stage can leave a child holding the
write end of the pipe.

Not a safety property. A program that finishes in time is no safer than one that does not.

`verified-by: none`

<a id="CMDLINE-14"></a>
### CMDLINE-14: a call may run in the background, and its finish wakes the turn

A call may say it should not be waited for. It is endorsed the same way, starts the same way, and
answers immediately with a handle. The turn carries on.

When it exits, the turn is told: the handle, the status, and the output under the label the plan
earned. A background run that is still going when the session ends is killed.

The output of a background run obeys every other clause here. Backgrounding changes when the planner
is told, never what it is allowed to read.

**Why this is a parameter and not the `&` token.** As syntax it would be one more thing the compiler
has to model and one more thing a reader has to spot in a line. As a parameter it is a field on the
call, visible in the prompt, and impossible to hide inside an argument.

`verified-by: none`

<a id="CMDLINE-15"></a>
### CMDLINE-15: a program that wants a terminal is refused before it starts

Standard input is empty, as it is today, so a program that reads it gets nothing rather than the
terminal. Beyond that, invocations known to require interaction are refused at compile time with a
message naming the alternative: `git rebase -i`, `git add -i`, an editor, a pager without
`--no-pager`, anything that would open `/dev/tty`.

The list is a convenience rather than a guarantee. Something interactive that is not on it hits the
deadline and returns what it printed, which is the same outcome by a slower road.

`verified-by: bravebot_agent::cmdline::a_program_that_wants_a_terminal_is_refused_before_it_starts`
`verified-by: bravebot_agent::cmdline::the_same_program_without_the_interactive_part_is_not_refused`
`verified-by: bravebot_agent::exec::a_stage_that_reads_stdin_is_given_nothing_rather_than_the_terminal`

<a id="CMDLINE-16"></a>
### CMDLINE-16: the tool's own description tells the planner to filter at the source

The description must say that narrowing a result inside the command is the cheapest thing the
planner can do, and must give the shapes: `| head -n`, `-l` for names only, `-c` for a count, a line
range rather than a whole file.

It must not tell the planner to prefer the dedicated read and search tools over a command line for
exploration. That instruction is in the current description, and it is backwards in a large tree: a
capped structured search returns thousands of tokens of truncated matches where `grep -rl` returns a
dozen lines, and the planner following the instruction pays the difference every round.

What the description should say instead is the true thing: use whichever returns less, and a command
that filters usually returns less.

**Why a clause about wording.** A tool's description is the only instruction the planner reliably
reads, and an instruction to reach for the structured tools instead measurably steers a session
into the expensive path. Wording that changes behaviour is behaviour.

`verified-by: bravebot_agent::tools::the_run_description_tells_the_planner_to_filter_at_the_source`
`verified-by: bravebot_agent::tools::the_run_description_does_not_send_the_planner_to_the_other_tools_instead`
`verified-by: bravebot_agent::tools::only_run_takes_a_command_line`

## Amendments to existing specs

This spec cannot land without these. Each is a real change to a normative clause and none should be
made quietly.

Each is named by what the clause says rather than by its id, because an id from another file means
nothing until you have opened that file.

| Spec | The clause | Change |
|---|---|---|
| [run.md](run.md) | `run` takes a pipeline of argv stages, never a command string | Amended. The planner's spelling becomes a command line; the *execution path* stays argv-only, which is the half that carries the property. The clause's second paragraph survives verbatim and is the load-bearing one. |
| [run.md](run.md) | argv is routing and must be endorsed by a person | Amended. Routing is the compiled plan rather than the argv the planner wrote. `(T,pub)` and exact binding are unchanged. |
| [run.md](run.md) | the table of what output is labelled | Extended. A fourth row: output of a read-proven plan, labelled from its read set. The default is untouched. |
| [run.md](run.md) | every run asks unless every stage was vouched for | **Narrowed by [CMDLINE-9](#CMDLINE-9).** "There is no read-only category" becomes "there is no *declared* read-only category, and one narrow *proven* one". This is the clause a reviewer is most likely to refuse, and the spec is still worth landing without it. |
| [run.md](run.md) | the vouched-for list is not an allowlist | Unchanged and reaffirmed. The audited table is not an allowlist either: it decides a label and a prompt, never whether something may run. Nothing is refused for being absent from it. |
| [run.md](run.md) | a run has a wall-clock limit, and reaching it ends the run rather than failing it | Superseded by [CMDLINE-13](#CMDLINE-13), which keeps that behaviour and makes the limit per call. |
| [shell-mode.md](../shell-mode.md) | the planner gets no shell tool, ever | **Amended, and this is the big one.** "The planner gets no shell tool" stands: no shell process is started for anything the planner wrote. What is withdrawn is the reading that also banned the notation. The clause should be restated as *the planner's line is never interpreted*, which is what it was protecting. |
| [permissions.md](../permissions.md) | every stage of a pipeline is judged on its own | Amended. Stages come from the compiler rather than from an array. Its closing sentence, that there is no shell to do the splitting and so this holds rather than being a matter of parsing carefully, must be replaced with the honest version: the splitting is done once, here, and nothing re-splits afterwards. |
| [tools/read-output.md](read-output.md) | letting a person release quarantined output | Unchanged, and used less. It remains the route for output no proof and no person has covered. |

## What is deliberately not adopted

- **Command substitution and parameter expansion.** [CMDLINE-2](#CMDLINE-2). These are the whole of
  what makes a shell string unendorsable, and adopting them would give up the property the rest of
  the spec is built to keep.
- **A fallback to a real shell when compilation fails.** The single change that voids this spec.
- **Aliases and profile loading.** A line's meaning must be in the line. Shell mode loads the user's
  syntax because a person typed it; nothing here does.
- **Confinement of children.** Out of scope and unchanged: programs run with the access the user's
  own shell would give them. That remains issue #4.
- **`git` in the audited table.** [CMDLINE-8](#CMDLINE-8).

## Open questions

- Whether [CMDLINE-9](#CMDLINE-9) is acceptable at all, or whether the prompt must always be a
  person's. The spec is deliberately written so that refusing it costs only the prompt and not the
  readable output.
- Whether a closed set of harness-supplied variables (workspace root, current directory) should be
  expanded at compile time. It is the same thing as the planner having written the value, so the
  objection in [CMDLINE-2](#CMDLINE-2) does not obviously reach it.
- Whether the read set should extend to a directory the plan walks but does not name, which is what
  `find . -name x` does. Naming `.` makes the read set the whole workspace, which is correct but
  coarse.
- Whether a background run's wake-up should be allowed to interrupt a round in flight or only land
  between rounds.

## Known costs

- **A compiler is a thing that can be wrong.** It is smaller than a shell and it is the only reader
  of the line, so a bug is a wrong plan rather than a divergence between two interpreters, but a
  wrong plan is still shown to a person as if it were right. The mitigation is that the plan, not
  the line, is what is displayed: a reader endorsing a plan that does not match the line they can
  see above it will notice.
- **The audited table is maintenance.** Every entry is a claim checked against one program's option
  surface, and option surfaces change with versions and with implementations. Keying on resolved
  path limits the blast radius; it does not remove the work.
- **More expressiveness is more that can be approved carelessly.** A one-line plan with eight stages
  and two redirections is harder to read at a prompt than `git log --oneline -50`. Bounding glob
  expansion helps; nothing fixes it entirely.
