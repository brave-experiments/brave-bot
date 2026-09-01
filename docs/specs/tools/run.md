---
id: RUN
title: run
status: normative
governs:
  - crates/agent/src/exec.rs
  - crates/core/src/command.rs
  - crates/core/src/programs.rs
  - crates/core/src/policy.rs
  - crates/tui/src/confirm.rs
guards:
  - symbol: Policy::read_output
  - symbol: Policy::remember_command
  - symbol: TrustedPrograms::trust
---

## Scope

`run`, its labels, and the two ways a person can change them. Shell mode is [shell-mode.md](../shell-mode.md) and is not an
instance of this: it is not a tool and the planner cannot reach it.

## Why a program is admissible when a shell is not

A shell string is destination and payload at once, so there is nothing in it a person could
approve on its own, and a parser that tried to work out what it means would be racing a shell it
does not control. An argument list has no such problem. This is the distinction to hold onto: it
is not that command execution turned out to be acceptable after all, it is that the exclusion was
about shell strings and an argv vector is not one.

## Clauses

<a id="RUN-1"></a>
### RUN-1: `run` takes a pipeline of argv stages, and never a command string

```
run { pipeline: [
  { program: "git", args: ["log", "--oneline", "-50"] },
  { program: "sed", args: ["-n", "1,10p"] }
]}
```

`; rm -rf /` in an argument is one argument and stays one, because nothing ever splits it. Pipes,
redirection, `&&`, globbing and `$(...)` are unavailable **to the planner**: each is a destination
nobody saw. Narrowing output is a stage, not a pipe character.

**The planner's execution path stays argv-only** and must never build a command line. It and shell
mode are separate modules, so a change to one cannot quietly become a shell for the other.

`verified-by: bravebot_agent::exec::a_metacharacter_in_an_argument_stays_one_argument`
`verified-by: bravebot_agent::exec::a_redirection_in_an_argument_writes_no_file`
`verified-by: bravebot_agent::exec::stages_are_chained_so_one_feeds_the_next`
`verified-by: bravebot_agent::exec::a_single_stage_returns_what_it_printed`
`verified-by: bravebot_core::policy::an_empty_pipeline_is_refused`

<a id="RUN-2"></a>
### RUN-2: argv is routing and must be endorsed by a person

Program and arguments must be `(T,pub)`. Untrusted text never becomes an argument. The
endorsement is bound to that exact argv, so it cannot be reused for a different one.

`verified-by: bravebot_core::policy::a_run_without_an_endorsement_is_refused`
`verified-by: bravebot_agent::exec::a_stage_runs_the_binary_it_was_resolved_to`
`verified-by: bravebot_agent::exec::a_pipeline_with_missing_resolutions_does_not_run`

<a id="RUN-3"></a>
### RUN-3: stdin is content and may be untrusted

The planner names a quarantined reference and the policy layer supplies the bytes, so `sed` and `awk`
work on a file nobody vouched for without the planner or the driver ever reading it. A stage that
reads stdin and was given none receives nothing, never the terminal.

**Why.** This is the point of the split: both trusted and untrusted data reach real tools, and
only the routing part has to be trustworthy.

`verified-by: bravebot_agent::exec::a_stage_that_reads_stdin_is_given_nothing_rather_than_the_terminal`

<a id="RUN-4"></a>
### RUN-4: output is untrusted and private by default, and nothing inferred changes that

| | Label | Gate |
|---|---|---|
| Program and arguments | `(T,pub)` | a person approves the exact argv |
| Standard input | may be untrusted | a person approves when it is private |
| Standard output and error | `(U,priv)` | quarantined |
| …for a command a person vouched for | `(T,priv)` | RUN-7 |

A program may print bytes an earlier stage read out of a file an attacker wrote, so `(U,priv)` is
the only label that holds without knowing what ran. Nothing a caller, a stage, or the planner can
declare changes it. Only a person can, in one of the two ways below, and both are assertions
rather than inferences.

`verified-by: bravebot_core::policy::output_nobody_vouched_for_is_untrusted_and_private`
`verified-by: bravebot_core::policy::output_of_a_vouched_command_is_trusted`
`verified-by: bravebot_core::policy::output_of_a_vouched_command_is_still_private`
`verified-by: bravebot_core::policy::one_unvouched_stage_makes_the_whole_output_untrusted`

<a id="RUN-5"></a>
### RUN-5: every run asks, unless every stage was vouched for

There is no read-only category. `foo --bar` might write to disk and nothing here can tell, and a
stage declaring itself harmless only helps if the declaration is honest. A person having answered
the question before, in this session, for this exact command is the **only** thing that may answer
it: never a property of the argv, never a declaration by a stage, never anything derived from what
a program printed.

**Why.** An unprompted write is worse than an unwanted prompt.

`verified-by: bravebot_core::policy::a_command_nobody_vouched_for_is_put_to_a_person`
`verified-by: bravebot_core::policy::a_vouched_command_is_not_asked_about_again`
`verified-by: bravebot_core::policy::one_unvouched_stage_puts_the_whole_pipeline_to_a_person`

<a id="RUN-6"></a>
### RUN-6: private input asks every time, whatever is vouched for

Untrusted input is fine, since carrying bytes decides nothing. Private input hands the user's data
to a program, and that releases it somewhere this policy stops governing. Trusted-but-private asks
too, and `a` is not offered for those runs at all.

**Why.** Vouching for what a file contains is not consenting to send it somewhere, and trusting a
command is not consenting to hand it the user's data.

`verified-by: bravebot_core::policy::private_input_asks_even_for_a_vouched_command`
`verified-by: bravebot_tui::confirm::a_run_that_releases_private_data_offers_no_standing_permission`
`verified-by: bravebot_tui::confirm::pressing_always_at_a_private_input_prompt_grants_nothing`

<a id="RUN-7"></a>
### RUN-7: vouching grants two things together, and the prompt asks for both

```
  y run it    a always    n don't    ctrl-c stop the turn
```

`a` grants, in these terms:

1. the command runs again unasked, side effects and all;
2. what it prints is `(T,priv)`, so the planner reads it instead of a reference.

The second is a **human assertion, not an inference**. Nothing establishes that a vouched command
is side-effect-free or that its output is free of influence, and nothing tries: `git log` prints
commit messages whoever contributed wrote. It is trusted for exactly the reason a directory in the
trust map is trusted, which is that the user said so. Do not reach for a stronger justification,
and do not let anything else mint an entry.

`verified-by: bravebot_tui::confirm::a_run_prompt_asks_for_the_side_effects_and_the_output_together`
`verified-by: bravebot_core::policy::a_turn_inherits_what_the_session_vouched_for`

<a id="RUN-8"></a>
### RUN-8: an entry is keyed by resolved path and exact arguments

`git log` says nothing about `git push`, and nothing about `git log --all`. `$PATH` and aliases
decide what a name means, so an assertion must not follow a name onto a different binary. Never
widen an entry to a program alone. In a pipeline **every** stage must be vouched for or the whole
output is untrusted, since an unvouched stage in the middle is a transformation nobody answered
for and its output is what the next stage read.

`verified-by: bravebot_core::policy::vouching_for_one_command_does_not_cover_another_of_the_same_program`
`verified-by: bravebot_core::policy::vouching_does_not_follow_a_name_onto_a_different_binary`

<a id="RUN-9"></a>
### RUN-9: the vouched list belongs to the session

Empty at the start of every session, written into the session record, restored by `--resume`,
never inherited by a fresh session in the same directory. `/status` lists what was granted.

**Why.** The same reason the trust map belongs to a session. It is the one permission whose whole effect is that
prompts stop, so it has to be readable back.

`verified-by: bravebot_core::policy::a_fresh_policy_vouches_for_no_command`

<a id="RUN-10"></a>
### RUN-10: the vouched-for list is not an allowlist and must never become one

It never decides what may run. A command nobody vouched for still runs after a prompt, nothing is
refused for being absent, and the set is empty at the start of every session. Programs are not
enumerated and not confined: they run with the access the user's shell would give them, because
`git push` needs `~/.ssh` and the set of programs someone might ask for cannot be listed in
advance.

Do not add an allowlist and treat it as the safety property. What holds is the label on the
output, not a belief about the binary.

`verified-by: none`

<a id="RUN-11"></a>
### RUN-11: a run has a wall-clock limit, and reaching it ends the run rather than failing it

A pipeline is given 300 seconds. When that runs out the stages are killed, and what they printed
before that is collected and returned exactly as it is for a pipeline that ended by itself, under
the label RUN-4 gives it. The stop is reported as structure, a duration on the result, so a
caller says which of the two happened without reading a byte of what was printed. Collecting after
a kill must be abandonable: a killed stage can leave a child holding the write end of the pipe, so
the run may not wait on that pipe reaching its end, and keeps whatever had been read by then.

**Why.** The limit is for the program that does not end, and the commonest such program is doing
exactly what was asked: a server told to serve a page serves it, prints as it goes, and never
exits. Returning the failure alone threw that account away and left a program that hung
indistinguishable from one that was working.

**Not a safety property.** A stage that finishes inside the limit is no safer than one that
outstays it, and nothing may be inferred about what a program did from the fact that it stopped in
time. This is a bound on futility: a program that never returns holds the turn open with nothing
to show for it. Being cut short neither raises nor lowers the label on the output, which is
RUN-4's to decide.

`verified-by: bravebot_agent::exec::a_pipeline_stopped_at_the_limit_still_returns_what_it_printed`
`verified-by: bravebot_agent::exec::a_pipeline_that_ends_by_itself_is_not_marked_stopped`
`verified-by: bravebot_agent::exec::a_grandchild_holding_the_pipe_does_not_hang_the_run`

## Open questions

- Whether to confine children is issue #4. Whether output can ever be trusted by proof rather than
  by assertion is issue #3. Neither may be resolved by weakening RUN-4.
- A separate proof path reaches RUN-4's trusted label by the other road, proving from the program
  and its arguments that a stage can read nothing the label does not account for. It is a proof about a program where
  RUN-7 is a person taking responsibility for one, and the two must not be merged. It remains
  unwired.
