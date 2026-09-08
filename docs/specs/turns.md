---
id: TURN
title: What bounds a turn
status: normative
governs:
  - crates/agent/src/turn.rs
---

## Scope

How long a turn may go on, what happens when it does not stop, and what is said when it goes on
without producing anything or ends without checking anything.

## Clauses

<a id="TURN-1"></a>
### TURN-1: a bounded turn loses its tools rather than ending

A turn carries a round limit or carries none. On the limiting round the next request offers no
tools, the planner is told it has none left, and it answers with what it has. A call it asks for
anyway is dropped rather than run.

**Not a safety property.** A gate refuses on the thousandth round what it refuses on the first.
This is a bound on futility: in a directory nobody vouched for, a planner looking for a file it
cannot name will try glob after glob for as long as anyone lets it. Do not cite this clause as a
containment measure.

**Compaction is not this bound.** Compaction bounds how full the context is, not how long a turn
runs. The glob loop above stays under any budget indefinitely, so compaction is what lets it run
forever rather than what stops it. The two cover opposite failures: compaction stops a turn dying,
this stops a turn never dying.

`verified-by: bravebot_agent::turn::a_turn_that_keeps_calling_tools_is_made_to_answer`
`verified-by: bravebot_agent::turn::calls_made_after_the_budget_is_spent_are_not_run`
`verified-by: bravebot_agent::turn::a_turn_is_not_cut_off_after_a_fixed_number_of_rounds`

<a id="TURN-2"></a>
### TURN-2: the bound belongs to the caller, and an interactive turn has none

Who is watching decides the limit, so the caller sets it. The terminal passes none: a person can
see what a turn is doing and a stop reaches it mid-round, so any number would only interrupt work
that was going fine. A one-shot `-p` run and a manifest run pass the default 200, because an
unwatched loop has nothing else to end it.

The default is bounded, because a default cannot know whether anybody is watching and being wrong
that way is the cheaper mistake. This was 40 everywhere, which interrupted real work in a large
repository.

`verified-by: bravebot_agent::turn::an_unbounded_turn_is_never_made_to_answer`
`verified-by: bravebot_agent::turn::a_turn_that_keeps_calling_tools_is_made_to_answer`

<a id="TURN-3"></a>
### TURN-3: a turn that has written nothing for long enough is told so

Where a write is possible and none has been asked for after a set number of rounds, the driver
says so once, at the end of a round, and the turn carries on with its tools. The line is a nudge,
not a bound: nothing is taken away, nothing is refused, and a planner that keeps reading keeps
reading.

**The number is measured, not chosen.** It was fifteen, and a run with the prompt and the line
together wrote its first file on round sixteen: the line working, and the paragraph asking for the
same thing not. Where a planner is reading past the point it could have written, the number is too
high.

**A different futility from [TURN-1](#TURN-1).** That one is about a turn which never ends. This
one is about a turn which ends having only understood: a planner that maps a repository before
changing anything is doing real work, and it still leaves nothing behind when somebody stops it,
which is the ordinary way a person finds out a turn went wrong.

**Said once, and conditionally worded.** Repeating it every round spends a request to say what is
already in the conversation. The driver cannot tell a task that asks for a change from one that
asks a question, and must not try: it knows only that rounds have gone by with nothing written,
so the line says what to do if a change was wanted and to carry on if it was not.

**A requested write counts, not a completed one.** A write the user refused is a planner that
tried to deliver, and telling it to start delivering would answer something nobody asked.

`verified-by: bravebot_agent::turn::a_turn_that_writes_nothing_for_long_enough_is_told_so`
`verified-by: bravebot_agent::turn::a_turn_that_has_written_is_not_told_to_write`

<a id="TURN-4"></a>
### TURN-4: a turn that changed files and ran nothing says so, to both parties

Where a run is possible, files have changed and no program has been run, the planner is asked once
whether any of it runs, a set number of rounds after its first write, and pointed at a checker
delegate for a long log. When the turn ends in that state the person is told plainly that nothing
was built or tested.

**Two audiences, two moments.** The planner can still act, so it is asked while the turn is going;
the person is about to act on a diff, so they are told at the end. Neither is a reproach: plenty of
turns have nothing to build, and both lines say what happened rather than what should have.

**Counted from the write.** Before a file changes there is nothing to run, so a turn that spends
twenty rounds reading is not asked about a build it has no reason to have done.

`verified-by: bravebot_agent::turn::a_turn_that_writes_without_running_is_asked_about_it`
`verified-by: bravebot_agent::turn::a_turn_that_wrote_and_ran_is_not_asked_about_it`
