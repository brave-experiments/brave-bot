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
crosses back to the planner is [delegation.md](delegation.md). Nothing here changes any of it:
the subject is what reaches a screen, not what reaches a model.

What is drawn for the turn itself is [terminal-transcript.md](terminal-transcript.md).

## Why it exists

A delegate's work is discarded by design: the planner is told a sentence, and the reading, the
commands and the narration behind it end with the delegate. Drawn nowhere, that leaves the person
with a single line about work they cannot see, done in a directory they own.

Several delegates run at once, so there is no single thing to look at. Each is drawn on the
screen the person is already reading, showing what it is doing.

## Clauses

<a id="WATCH-1"></a>
### WATCH-1: a delegate's work is drawn under the delegate, and never among the turn's lines

Each one has a block of its own, where the call that started it happened, and what it reads and
runs is drawn inside that block. Nothing of it is drawn in the turn's own sequence.

**Why.** Interleaved, neither sequence can be read: a turn that asked a delegate to run the build
would have the build log in the middle of it, which is the context problem the whole design
exists to avoid, reappearing on the screen. With several delegates running it is also ambiguous,
since two of the same kind produce identical lines and nothing on the row says which run touched
which file.

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

**Why.** The whole of a delegate's work is what delegating exists to absorb, so keeping it on the
screen would mean holding a second transcript of every run. Three rows under a delegate that has
made thirty calls read as a delegate doing very little, which is what the count answers.

`verified-by: bravebot_tui::state::a_delegates_block_keeps_the_last_of_its_work_and_counts_the_rest`
`verified-by: bravebot_tui::render::a_delegate_that_has_done_more_than_is_drawn_says_so`

<a id="WATCH-4"></a>
### WATCH-4: a delegate that has finished collapses to what the turn was told

Its block ends on that sentence and stops drawing the work behind it.

**Why.** What anybody acts on is the conclusion, and a block that stops without saying how it
ended leaves a reader looking at a last tool call, unable to tell an answer from a failure.
Several blocks left open at their last command also spend rows on work that is over.

`verified-by: bravebot_tui::state::what_a_delegate_ended_with_closes_its_block`
`verified-by: bravebot_tui::render::a_finished_delegate_collapses_to_what_the_turn_was_told`

<a id="WATCH-5"></a>
### WATCH-5: a reply a delegate is writing is drawn nowhere

What a delegate writes between its tool calls is not drawn. Its conclusion reaches the screen as
the report, which is the sentence its block ends on.

**Why.** The turn's own half-written reply is drawn at the tail of the screen. A delegate's
sentence drawn there would read as the planner writing something it never wrote.

`verified-by: bravebot_tui::state::a_reply_a_delegate_is_writing_is_not_drawn_over_the_turn`

<a id="WATCH-6"></a>
### WATCH-6: none of this reaches a model, and none of it is written down

A delegate's lines go to a screen and stop there. The planner that asked is told the report and
nothing else, and no block is part of the record a session is resumed from, so a resumed session
has no delegates in it. Starting a new conversation forgets them.

**Why.** A screen is not a context. The person owns the directory and may see what their agent
did in it; what must not happen is those lines reaching a planner's context by any route. A
record read back into a later turn is such a route, which is why nothing here is written down.

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
