---
id: MODE
title: How much a session asks before it acts
status: normative
governs:
  - crates/agent/src/permission_mode.rs
  - crates/agent/src/delegate.rs
  - crates/cli/src/main.rs
  - crates/tui/src/state.rs
guards:
  - symbol: Confining
  - symbol: PermissionMode
  - symbol: Task::with_permission_mode
  - symbol: Session::allowing_bypass
  - symbol: Session::cycle_permission_mode
---

## Scope

A standing answer to the questions a person is otherwise asked one at a time. Four modes: asking,
accepting edits, planning, and bypassing every check. One key cycles them and the mode in force is
drawn under the input box for as long as it holds.

What each of those questions *is*, and what a prompt owes its reader, is
[prompting.md](prompting.md). This spec covers only who answers them and what an answer amounts to
when nobody is asked.

A mode is not a rule. Rules written in advance in the settings file are
[permissions.md](permissions.md), they decide whether there is anything to prompt about, and a mode
answers a prompt that a rule has already permitted to exist. The two meet in MODE-6.

## What a mode decides

<a id="MODE-1"></a>
### MODE-1: asking is the mode a session opens in, and the only one that puts every effect to a person

A session with nothing chosen asks about a write, a run, a command's output, and a file nobody
vouched for, exactly as one does where no mode can be chosen at all. A one-shot run is in this mode
unless the flag in MODE-5 was given.

**Why.** The mode nobody selected cannot be one that stops asking. Every other mode here is a
decision somebody made; this one is what holds when they have made none.

`verified-by: bravebot_agent::permission_mode::the_default_mode_asks_about_everything`
`verified-by: bravebot_tui::state::a_session_starts_by_asking_about_everything`

<a id="MODE-2"></a>
### MODE-2: accepting edits answers the write prompt and no other

A write goes through unasked. A run, a command's output and a file nobody vouched for are still put
to the person.

**Why.** A write and a run are not the same risk. A write lands in a tree the person can read
afterwards, and `git diff` shows them all of it; a program runs with everything their own shell has,
leaves no diff, and its output is what the next stage reads. A mode named for edits that also
stopped asking about programs would be granting the larger thing quietly.

`verified-by: bravebot_agent::permission_mode::accepting_edits_lets_writes_through_but_not_commands`

<a id="MODE-3"></a>
### MODE-3: plan mode refuses a write rather than asking about one

The refusal does not depend on how the person would have answered: a write is refused where the
prompt would have been approved. Commands, output and vouching are asked about as they are in every
other mode.

The planner is told, in the system prompt, that writing is refused for the turn and why.

**Why.** A mode whose only difference was that somebody keeps saying no is a session where the
planner proposes writes and reads back refusals it cannot account for, and a planner that cannot
tell a policy from a mistake retries. Stating it once is what turns a series of refusals into a
constraint the planner can work inside.

Commands keep their prompt because research is most of what planning is. `git log`, a search and a
test run are how a plan is arrived at, and a mode that could not read the tree would produce plans
made from less than the person can see themselves.

`verified-by: bravebot_agent::permission_mode::plan_mode_refuses_a_write_the_person_would_have_approved`
`verified-by: bravebot_agent::permission_mode::plan_mode_still_lets_a_command_be_asked_about`
`verified-by: bravebot_agent::permission_mode::only_plan_mode_says_anything_to_the_planner`
`verified-by: bravebot_agent::turn::plan_mode_writes_nothing_even_where_writes_are_approved`

<a id="MODE-4"></a>
### MODE-4: bypassing answers all four questions, including the one that lifts quarantine

A write, a run, a command's output and vouching for a file nobody vouched for are all approved
without being put to anybody. The last is the one that costs the most: vouching is what lets a
file's contents be shown to the planner rather than held behind a reference, so in this mode every
file the planner asks to read is shown to it.

A run approved this way vouches for no program. The list of commands a person said to stop asking
about is written into the session record and outlives the mode, and a record claiming somebody
approved programs they were never shown would be a standing permission nobody granted.

**Why.** The mode is for a place where the blast radius is bounded by something other than these
prompts, which in practice means a container with no network and nothing in it worth losing. It is
the wrong mode everywhere else, and it is named `--dangerously-skip-permissions` for that reason.

`verified-by: bravebot_agent::permission_mode::bypassing_answers_every_permission_question`

## Choosing one

<a id="MODE-5"></a>
### MODE-5: bypassing is reachable only where the command line asked for it

`--dangerously-skip-permissions` does two things and they are inseparable: it opens the session in
that mode, and it puts that mode on the ladder the key walks. Without it the mode cannot be reached
however many times the key is pressed.

The flag is taken out of the arguments before anything dispatches on them, so it composes with a
bare invocation, `-p`, `--resume`, `--continue`, `--mode` and `--incognito` alike, and repeating it
asks for the same thing once. A one-shot run has no key to press, so the flag is the whole of what
can say.

**Why.** A mode that answers every question has to be asked for where the asking is recorded, which
is the command line somebody typed. Honouring only the second half would leave the flag doing
nothing a person could see, and disagreeing with what the same flag does to a one-shot run.

`verified-by: bravebot_agent::permission_mode::bypass_is_only_reachable_where_the_flag_was_given`
`verified-by: bravebot_tui::app::the_key_cannot_reach_bypass_without_the_flag`
`verified-by: bravebot_tui::app::the_flag_opens_the_session_in_bypass_and_can_be_cycled_out_of`
`verified-by: bravebot_cli::main::permissions_are_enforced_unless_the_flag_is_given`
`verified-by: bravebot_cli::main::the_skip_permissions_flag_is_taken_out_wherever_it_appears`
`verified-by: bravebot_cli::main::the_flag_leaves_every_other_way_of_starting_intact`
`verified-by: bravebot_cli::main::it_composes_with_incognito`

<a id="MODE-6"></a>
### MODE-6: no mode answers a question a rule already refused

A deny rule from the settings file holds in every mode, including the one that asks about nothing. It
refuses before there is anything to prompt about, so there is no prompt for a mode to answer. A mode
stops the asking; it does not discard what somebody wrote down.

**Why.** This looks like the mode working, which is what makes it worth stating. A flag that quietly
undid a `deny` rule would take protection away at the moment somebody was relying on a mode to save
them keystrokes, and the file they wrote it in is the place they would go to check.

`verified-by: bravebot_agent::turn::a_deny_rule_holds_where_every_permission_check_is_bypassed`

<a id="MODE-7"></a>
### MODE-7: no mode answers a question that is not a permission

A question the planner posed, and a line the person typed unprompted, reach the person in every mode.
Neither asks for consent: the first asks for information, and the second is the person speaking.

**Why.** An answer invented on somebody's behalf is reported to the planner as their own words, so a
mode that answered these would be putting words in the mouth of a person sitting in front of the
session. That a mode stops prompts is not a reason to think they wanted this one stopped.

`verified-by: bravebot_agent::permission_mode::no_mode_answers_a_question_that_is_not_a_permission`

<a id="MODE-8"></a>
### MODE-8: a turn keeps the mode it began with, and both halves read one value

The mode is read once, when the prompt is sent. The planner is told that mode and the prompts are
answered against that mode, so the two cannot disagree about a turn in flight. A press while a turn
runs describes the next turn.

**Why.** A write already on screen being reviewed must not have the question withdrawn from under
the person answering it. Reading the mode in two places at two moments is how the planner comes to
be told one thing while the prompts do another, and the half that would be wrong is the half nobody
can see.

`verified-by: bravebot_tui::app::the_mode_can_be_changed_while_a_turn_runs`

<a id="MODE-9"></a>
### MODE-9: a delegate inherits the mode of the turn that spawned it

Both halves: the prompts a delegate's work raises are answered against that mode, and the delegate's
own planner is told it.

**Why.** A delegate is the spawning turn's work done somewhere else, so a session that is planning
must not write through one. The enforcing half holds because a delegate's prompts travel back to the
same person; the telling half has to be passed deliberately, and without it a delegate's planner
reads refusals it cannot account for and retries.

`verified-by: bravebot_agent::turn::a_delegate_inherits_the_mode_of_the_turn_that_spawned_it`

<a id="MODE-10"></a>
### MODE-10: a mode belongs to the sitting it was chosen in

A mode is not written into the session record. A resumed session opens by asking, whatever the
session that wrote the record was doing when it ended, and `--resume` with the flag in MODE-5 opens
in bypass because the flag was given again.

**Why.** The two standing grants a resume restores, the trust map and the vouched commands, are
decisions about the future that their owner made deliberately ([sessions.md](sessions.md) is what
keeps them). A mode is an answer somebody gave while watching one piece of work. Coming back
tomorrow into a session that had stopped asking about writes, with nothing chosen today, is the
wrong direction for this to be wrong in.

`verified-by: bravebot_tui::state::cycling_the_mode_changes_nothing_a_resume_would_read`
`verified-by: bravebot_tui::sessions::a_resumed_session_asks_about_writes_whatever_the_record_says`

## Known costs

- **Accepting edits accepts a write to any path the workspace reaches.** The mode answers the write
  prompt, and the prompt is the only thing that would have shown the person the path. A rule in the
  settings file is what narrows it, and a `deny` rule still holds (MODE-6); the mode itself does not
  distinguish one file from another.
- **Bypassing gives up injection containment for files that are read.** Vouching is what decides
  whether a file's contents are shown to the planner or held behind a reference, so in that mode a
  file holding instructions rather than data is read as instructions. The guarantee that untrusted
  content cannot *decide* what happens is structural and still holds; what goes is the narrower
  protection of not showing the planner bytes nobody vouched for. This is why the mode is for a
  sandbox rather than for a working machine.
- **Plan mode constrains writes, not everything a turn can do.** Commands are still asked about, and
  an approved command may write whatever it likes: it runs with the access the person's own shell
  has. The mode refuses the write tools rather than making the turn incapable of changing anything.
  [sandboxing.md](sandboxing.md) is what confines a process.
- **`defaultMode` in the settings file selects no mode.** The key is parsed so a file carrying it is
  not rejected, and a person who wrote `acceptEdits` there gets the prompts they would have got
  without it. The command line and the mode key are what choose a mode.
