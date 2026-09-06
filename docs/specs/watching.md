---
id: WATCH
title: Seeing a delegate work
status: normative
governs:
  - crates/tui/src/state.rs
  - crates/tui/src/render.rs
  - crates/tui/src/app.rs
---

## Scope

What a person sees of the delegates a turn started: where their lines go, how much of them is
kept, the mode Ctrl-L opens over them, and what that mode does not offer. What a delegate is, how
many run at once and what crosses back to the planner is [delegation.md](delegation.md). Nothing
here changes any of it: the subject is what reaches a screen, not what reaches a model.

What is drawn for the turn itself is [terminal-transcript.md](terminal-transcript.md). Reading
back through what has already happened is [scroller.md](scroller.md), whose keys this mode borrows
rather than inventing a second dialect of.

## Why it exists

A delegate's work is discarded by design: the planner is told a sentence, and the reading, the
commands and the narration behind it end with the delegate. Drawn nowhere, that leaves the person
with a single line about work they cannot see, done in a directory they own.

Several delegates run at once, so there is no single thing to look at. Each is drawn on the
screen the person is already reading, and the whole of what any one of them is doing is a key
away.

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
### WATCH-3: the block draws the last few of a delegate's work and counts the rest

Three of its calls are drawn where it started, and a delegate that has made more than three says
how many it has made.

**Why.** The turn's own sequence is what the block sits in, and a delegate that makes thirty calls
would otherwise push the turn off the screen. Three rows without the count read as a delegate
doing very little, which is what the count answers.

`verified-by: bravebot_tui::state::a_delegates_block_draws_the_last_of_its_work_and_counts_the_rest`
`verified-by: bravebot_tui::render::a_delegate_that_has_done_more_than_is_drawn_says_so`

<a id="WATCH-4"></a>
### WATCH-4: what the block has no room for is kept, up to a bound

A delegate holds its work beyond the few its block draws, and drops its oldest once it has made
several hundred calls.

**Why.** The block is a glance and the mode is the reading, so keeping only what the block draws
would leave the mode with three rows to show for an hour's work. The bound is there because a
delegate runs as long as a turn does, and this is held in memory for a person who may never look.

`verified-by: bravebot_tui::state::a_delegate_keeps_the_work_its_block_has_no_room_for`
`verified-by: bravebot_tui::state::a_delegate_stops_keeping_its_oldest_work`

<a id="WATCH-5"></a>
### WATCH-5: a delegate that has finished collapses to what the turn was told

Its block ends on that sentence and stops drawing the work behind it. The work is still there to
be opened.

**Why.** What anybody acts on is the conclusion, and a block that stops without saying how it
ended leaves a reader looking at a last tool call, unable to tell an answer from a failure.
Several blocks left open at their last command also spend rows on work that is over.

`verified-by: bravebot_tui::state::what_a_delegate_ended_with_closes_its_block`
`verified-by: bravebot_tui::render::a_finished_delegate_collapses_to_what_the_turn_was_told`

<a id="WATCH-6"></a>
### WATCH-6: a reply a delegate is writing is drawn nowhere

What a delegate writes between its tool calls is not drawn, in its own view or the turn's. Its
conclusion reaches the screen as the report.

**Why.** The turn's own half-written reply is drawn at the tail of the screen. A delegate's
sentence drawn there would read as the planner writing something it never wrote, and the same
sentence drawn in a delegate's view would be the only thing on that screen the delegate did not
do.

`verified-by: bravebot_tui::state::a_reply_a_delegate_is_writing_is_not_drawn_over_the_turn`

<a id="WATCH-7"></a>
### WATCH-7: Ctrl-L opens the delegates: the list where there are several, the one where there is one

The list is the way in where the session has spawned more than one, and where it has spawned one
that delegate's own lines are what opens. The delegate the view opens on is the one that is
working, or the most recent where none is.

Where the session has spawned none the key does nothing at all.

**Why.** Which delegate is the question a person has when several are going, and a mode that
opened straight into one of them answers a question they did not ask. A list of one is a row to
press through to reach the only thing behind it. A mode that opens on an empty screen is worse
than a key that does not answer: it puts somebody somewhere, with nothing to read and something to
get out of.

`verified-by: bravebot_tui::app::ctrl_l_watches_the_delegate_that_is_working`
`verified-by: bravebot_tui::app::ctrl_l_does_nothing_where_no_delegate_has_run`
`verified-by: bravebot_tui::app::enter_opens_the_delegate_the_list_is_on`
`verified-by: bravebot_tui::state::watching_opens_on_the_delegate_that_is_working`
`verified-by: bravebot_tui::state::there_is_nothing_to_watch_until_a_delegate_has_run`
`verified-by: bravebot_tui::state::several_delegates_are_opened_on_the_list_of_them`
`verified-by: bravebot_tui::state::one_delegate_is_opened_without_a_list_to_pick_from`

<a id="WATCH-8"></a>
### WATCH-8: a delegate's view opens on what it was asked and closes on what it answered

What is drawn is the delegate's work and none of the turn's. What it was asked to do stands above
its lines, and the sentence the turn was told closes them.

`n` and `p` move between delegates without going back to the list, and stop at each end rather
than wrapping. Coming out puts the turn's own view back where it was left.

**Why.** The view is read by somebody who was not told what the delegate was asked to do, and a
view that stopped at the last call leaves them looking at a command, unable to tell an answer from
a failure. Comparing two runs is what having several is for, and stepping through wants to arrive
at the last one and know that it is the last.

`verified-by: bravebot_tui::state::watching_a_delegate_shows_its_lines_rather_than_the_turns`
`verified-by: bravebot_tui::state::moving_between_delegates_stops_at_each_end`
`verified-by: bravebot_tui::state::coming_back_from_a_delegate_puts_the_turns_view_where_it_was_left`
`verified-by: bravebot_tui::state::the_turns_view_is_not_dragged_by_reading_through_a_delegate`
`verified-by: bravebot_tui::render::a_delegates_view_draws_its_own_lines_and_not_the_turns`
`verified-by: bravebot_tui::render::a_finished_delegates_view_ends_on_what_the_turn_was_told`
`verified-by: bravebot_tui::render::a_delegates_view_does_not_open_on_the_mark`
`verified-by: bravebot_tui::app::n_and_p_move_between_delegates`

<a id="WATCH-9"></a>
### WATCH-9: the view takes every key, and the way out is read against the nearest level

Nothing falls through to the input box, and the box is not drawn. `q` and Escape go back to the
list from a delegate, and close the mode from the list or where there is no list behind it.
Ctrl-L and Ctrl-C close it from either level and do nothing else: the turn in flight goes on, and
the press that reaches it is the next one.

There is no key for talking to a delegate and no box for it. A delegate is given one task, has
nobody to ask, and takes no line typed mid-turn.

**Why.** What a person types while watching would otherwise wait in a line they cannot see, to be
sent to a turn they are not looking at. A person stops the nearest thing, and somebody who went to
look at what a delegate was doing is not asking for the turn to end when they come back out;
watching is also the mode most likely to be open while something is going wrong.

`verified-by: bravebot_tui::app::a_typed_character_does_not_reach_the_box_while_a_delegate_is_watched`
`verified-by: bravebot_tui::app::q_goes_back_to_the_list_before_it_closes`
`verified-by: bravebot_tui::app::q_closes_outright_where_there_is_no_list_to_go_back_to`
`verified-by: bravebot_tui::app::the_view_answers_the_stop_keys_before_the_turn_does`

<a id="WATCH-10"></a>
### WATCH-10: what is on the screen changes when a person asks, and not otherwise

A delegate finishing leaves the view on it. A delegate starting does not take the screen from
somebody reading an older one.

**Why.** Several delegates report at once, so a view that followed the newest event would move
under the reader several times a second, and the run somebody opened would be the one run they
could not keep on the screen.

`verified-by: bravebot_tui::state::a_delegate_that_finishes_is_still_the_one_being_watched`
`verified-by: bravebot_tui::state::a_new_delegate_does_not_take_the_screen_from_the_one_being_read`

<a id="WATCH-11"></a>
### WATCH-11: the footer speaks in the interface's own words, and the turn's own row names the key

The footer names the kind and the driver's number for it, says whether the delegate is working,
answered or did not finish, and names the way out. Nothing a model wrote is quoted there: what the
delegate was asked stands above the lines, where the conventions for drawing content apply. The
position and the keys for moving between delegates appear only where there is more than one.

While a delegate is working, the row saying what the turn is doing names the key, and the key is
in the shortcut list.

**Why.** The turn's transcript shows one block and three rows of it, so somebody who does not
already know the key has no way to find out there is anything more to see.

`verified-by: bravebot_tui::render::the_footer_says_which_delegate_this_is_and_whether_it_is_working`
`verified-by: bravebot_tui::render::one_delegate_is_given_no_position_and_no_key_for_moving`
`verified-by: bravebot_tui::render::the_list_names_every_delegate_and_what_each_was_asked`
`verified-by: bravebot_tui::render::a_narrow_row_keeps_the_count_and_loses_the_end_of_the_task`
`verified-by: bravebot_tui::render::the_indicator_says_which_key_watches_a_delegate_at_work`
`verified-by: bravebot_tui::render::the_shortcut_list_names_the_key_that_watches`

<a id="WATCH-12"></a>
### WATCH-12: none of this reaches a model, and none of it is written down

A delegate's lines go to a screen and stop there. The planner that asked is told the report and
nothing else, and no delegate is part of the record a session is resumed from, so a resumed
session has no delegates in it. Starting a new conversation forgets them, and the mode standing
over one closes with them.

**Why.** A screen is not a context. The person owns the directory and may see what their agent
did in it; what must not happen is those lines reaching a planner's context by any route. A
record read back into a later turn is such a route, which is why nothing here is written down.

`verified-by: bravebot_agent::turn::what_a_delegate_read_never_reaches_the_planner_that_asked`
`verified-by: bravebot_tui::state::clearing_forgets_the_delegates`
`verified-by: bravebot_tui::state::clearing_closes_the_view_over_a_delegate`
`verified-by: by-construction (a session's record is built from the conversation, which holds the turn's messages; a delegate's lines are held only by the interface and nothing writes them)`

## Known costs

- **A delegate that runs long enough loses its oldest work.** Several hundred calls in, the start
  of a run is gone from the screen, and a person arriving late reads from wherever the bound has
  reached. The alternative is holding the whole of every delegate for the length of a session, for
  a reader who may never open one.

- **A delegate cannot be seen after the session that started it.** The record holds the
  conversation, and a delegate's exchange is deliberately not in it, so resuming brings back the
  report and none of the work behind it.

- **The view says what a delegate is doing and not how it is going.** One row changes several
  times a second while a delegate works, and nothing on the screen says whether those calls are
  getting anywhere. The report says, and it says it at the end.
