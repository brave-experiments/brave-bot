---
id: WATCH
title: Seeing a delegate work
status: normative
governs:
  - crates/tui/src/state.rs
  - crates/tui/src/render.rs
---

## Scope

What a person sees of the delegates a turn started: where their lines go, how much of them is
kept, and what is left when one finishes. What a delegate is, how many run at once and what
crosses back to the planner is [delegation.md](delegation.md), and none of it changes here: this
is the same run reaching a person's eyes rather than a model's context.

What is drawn for the turn itself is [terminal-transcript.md](terminal-transcript.md).

## Why it exists

A delegate is the one part of a run whose work is deliberately thrown away. The planner is told a
sentence, and everything behind it, the reading, the commands, the narration, ends with the
delegate. That is the feature. It also means the person watching had nothing at all: one line
saying a delegate answered, about work they never saw, in a workspace they own.

Several of them work at once, so there is no one thing to look at. What a person needs is all of
them on the screen they are already reading, each saying what it is doing now.

## Clauses

<a id="WATCH-1"></a>
### WATCH-1: a delegate's work is drawn under the delegate, and never among the turn's lines

Each one has a block of its own, where the call that started it happened, and what it reads and
runs is drawn inside that block. Nothing of it is drawn in the turn's own sequence.

**Why.** Interleaved, neither sequence can be read. The turn that asked a delegate to run the
build would have the build log in the middle of it, which is the context problem the whole design
exists to avoid, reappearing on the screen. With several delegates going it is worse than
unreadable: two of the same kind produce lines that are identical, so a person could not tell
which run had touched which file.

`verified-by: bravebot_tui::state::a_delegates_work_goes_under_its_own_block_and_not_into_the_turns_lines`
`verified-by: bravebot_tui::state::each_delegates_work_lands_under_the_delegate_that_did_it`
`verified-by: bravebot_tui::render::every_delegate_is_drawn_with_its_own_work_under_it`

<a id="WATCH-2"></a>
### WATCH-2: which block a line goes in is what the driver said, never what the line says

A report lands under the delegate the driver named as its author, and under the turn where it
named none. Nothing reads a line to work out whose it is, and where a line arrived in the
sequence decides nothing.

**Why.** A line is prose a model had a hand in. An interface deciding from one which run it
belonged to would be taking that decision from model output, which is the thing this repository
refuses everywhere else. Order cannot stand in for it either: several runs report at once, so the
order lines arrive in is the order the work happened rather than the order it was asked for.

`verified-by: bravebot_tui::state::each_delegates_work_lands_under_the_delegate_that_did_it`
`verified-by: bravebot_tui::state::the_turns_own_lines_come_back_once_a_delegate_has_finished`

<a id="WATCH-3"></a>
### WATCH-3: the block is a live view of what one is doing, not a transcript of what it did

The last few of its calls are kept and the rest are counted, so a block says both what is
happening now and how much has happened. A delegate that has done more than is drawn says so.

**Why.** The whole of a delegate's work is what delegating exists to absorb, and keeping it on
the screen would be keeping a second transcript of every run whose purpose was to be forgotten.
Three rows under a delegate that has made thirty calls would read as a delegate doing very
little, which is why the count is there.

`verified-by: bravebot_tui::state::a_delegates_block_keeps_the_last_of_its_work_and_counts_the_rest`
`verified-by: bravebot_tui::render::a_delegate_that_has_done_more_than_is_drawn_says_so`

<a id="WATCH-4"></a>
### WATCH-4: a delegate that has finished collapses to what the turn was told

Its block ends on that sentence and stops drawing the work behind it.

**Why.** What anybody acts on is what it concluded, and a block that stopped without saying how
leaves somebody looking at a last tool call, unable to tell an answer from a failure. Several
blocks sitting open at their last command would also be several delegates' worth of rows for work
that is over.

`verified-by: bravebot_tui::state::what_a_delegate_ended_with_closes_its_block`
`verified-by: bravebot_tui::render::a_finished_delegate_collapses_to_what_the_turn_was_told`

<a id="WATCH-5"></a>
### WATCH-5: a reply a delegate is writing is drawn nowhere

A delegate says a great deal on its way to an answer, and none of it is drawn. What it concluded
arrives as the report, which is the sentence its block ends on.

**Why.** The turn's own half-written reply is drawn at the tail of the screen, and one model
writes at a time. A delegate's sentence appearing there would read as the planner writing
something it never wrote.

`verified-by: bravebot_tui::state::a_reply_a_delegate_is_writing_is_not_drawn_over_the_turn`

<a id="WATCH-6"></a>
### WATCH-6: none of this reaches a model, and none of it is written down

A delegate's lines go to a screen and stop there. The planner that asked is told the report and
nothing else, and no block is part of the record a session is resumed from, so a resumed session
has no delegates in it. Starting a new conversation forgets them.

**Why.** This is the clause that keeps the view from undoing what delegating is for. A screen is
not a context: the person owns the workspace and is entitled to see what their agent did in it.
What must not happen is those lines reaching a planner's context by any route, and a record read
back into a later turn is exactly such a route.

`verified-by: bravebot_agent::turn::what_a_delegate_read_never_reaches_the_planner_that_asked`
`verified-by: bravebot_tui::state::clearing_forgets_the_delegates`
`verified-by: by-construction (a session's record is built from the conversation, which holds the turn's messages; a delegate's block is held only by the interface and nothing writes it)`

## Known costs

- **The whole of a delegate's work cannot be read.** A block keeps the last few calls, so a
  person who looks away while one is busy comes back to a count and the newest rows. Keeping the
  rest would mean holding a second transcript for every run whose purpose was to be forgotten,
  and there is no screen to put it on that is not the one the turn is using.

- **A delegate cannot be seen after the session that started it.** The record holds the
  conversation, and a delegate's exchange is deliberately not in it, so resuming brings back the
  report and none of the work behind it.

- **A block says what a delegate is doing and not how it is going.** One row changes several
  times a second while a delegate works, and nothing on the screen says whether those calls are
  getting anywhere. The report says, and it says it at the end.
