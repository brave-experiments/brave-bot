---
id: LOOP
title: Repeating a prompt
status: normative
governs:
  - crates/tui/src/loops.rs
  - crates/tui/src/app.rs
  - crates/tui/src/state.rs
guards:
  - symbol: Session::start_loop
  - symbol: Running::dispatched
---

## Scope

`/loop`: sending one prompt again and again until somebody stops it. What the repeated line is,
where the interval comes from, when a tick fires, and what ends a loop.

Not what a tick then does, which is a turn like any other. The tool a self-paced tick uses to say
when the next one is due is [tools/schedule-next.md](tools/schedule-next.md), and what the planner
is told about being inside a loop is a skill, in [skills.md](skills.md). That a `/` line is a
command at all, and that only a key press produces one, is [commands.md](commands.md).

## What repeats

<a id="LOOP-1"></a>
### LOOP-1: the line a loop repeats is the one the person typed, for as long as it runs

The argument to `/loop`, with any interval taken off it, is settled when Enter is pressed and is
sent unchanged on every tick. Nothing a turn reads, writes, says or returns can add to it, edit
it or replace it. A planner may say *when* the next tick happens and there is nowhere for it to
say what the next tick asks.

**Why.** A schedule that a turn could write the next prompt into is a turn that can rewrite its
own instructions, and the whole point of a loop is that it asks the same question again. The
person endorsed a line; every tick has to be that line.

`verified-by: bravebot_tui::app::the_loop_command_sends_what_is_left_after_the_interval`
`verified-by: bravebot_tui::state::the_first_tick_of_a_loop_goes_immediately`
`verified-by: bravebot_agent::turn::a_tick_of_a_self_paced_loop_says_when_to_run_again`

<a id="LOOP-2"></a>
### LOOP-2: a tick is a prompt, never a command

`/loop 5m /status` sends the seven characters `/status` to the planner every five minutes. It
does not run the status command, and no argument to `/loop` ever reaches the command dispatcher.

**Why.** A command is dispatched from a key press and from nothing else, and a timer is not a key
press. A loop that could run commands would be a way to make this program act on a schedule
nobody was watching, with each firing endorsed by a keystroke made once, long ago.

`verified-by: bravebot_tui::state::a_loop_whose_prompt_looks_like_a_command_still_sends_it_as_a_prompt`

<a id="LOOP-3"></a>
### LOOP-3: an interval is read off the front of the argument, or off an `every` clause at its end

In that order, and nowhere else.

| The argument | The interval | What is sent |
|---|---|---|
| `5m check the deploy` | 5 minutes | `check the deploy` |
| `check the deploy every 20m` | 20 minutes | `check the deploy` |
| `check the deploy every 20 minutes` | 20 minutes | `check the deploy` |
| `check every PR` | none, so each turn paces it | `check every PR` |
| `5m check the deploy every 20m` | 5 minutes | `check the deploy every 20m` |
| `5m`, or nothing at all | there is nothing to send | nothing; the command says what it needs |

A leading token counts only when it is a number and one of `s`, `m`, `h` or `d` and nothing
else. A trailing clause counts only when a time expression is the whole of what follows `every`,
which is what keeps `check every PR` a sentence rather than a sentence with its last two words
taken off.

`verified-by: bravebot_tui::loops::an_interval_written_first_is_taken_off_the_front`
`verified-by: bravebot_tui::loops::every_unit_letter_is_understood_at_the_front`
`verified-by: bravebot_tui::loops::an_interval_written_last_is_taken_off_the_end`
`verified-by: bravebot_tui::loops::a_trailing_interval_may_spell_its_unit_out`
`verified-by: bravebot_tui::loops::every_without_a_time_after_it_is_words_rather_than_an_interval`
`verified-by: bravebot_tui::loops::a_leading_interval_wins_over_a_trailing_one`
`verified-by: bravebot_tui::loops::a_line_with_no_interval_is_paced_by_the_planner`
`verified-by: bravebot_tui::loops::a_word_that_is_not_a_time_stays_part_of_the_prompt`
`verified-by: bravebot_tui::loops::a_count_too_large_to_be_a_duration_is_not_an_interval`
`verified-by: bravebot_tui::loops::an_interval_with_nothing_to_send_is_not_a_request`
`verified-by: bravebot_tui::app::the_bare_loop_command_is_still_the_command`

## When a tick fires

<a id="LOOP-4"></a>
### LOOP-4: the first tick goes immediately

Starting a loop sends its prompt at once rather than waiting out the first interval.

**Why.** Somebody who has just asked for something every five minutes wants to see it happen
once, while they are still watching, and decide whether it was the right thing to ask for. A loop
whose first sign of life is five minutes of nothing is one nobody can tell is running.

`verified-by: bravebot_tui::state::the_first_tick_of_a_loop_goes_immediately`

<a id="LOOP-5"></a>
### LOOP-5: the gap is measured from the end of a tick, not from its start

`every 5m` means five minutes between runs.

**Why.** Measured from the start, a turn that outlasts its own interval would be due again the
instant it drew breath, and a loop over slow work would become a continuous one. The interval is
how often somebody wants to be told something, and telling them takes time too.

`verified-by: bravebot_tui::loops::a_tick_in_flight_is_not_due_again`
`verified-by: bravebot_tui::state::a_tick_that_says_when_to_wake_arms_the_next_one`

<a id="LOOP-6"></a>
### LOOP-6: a tick waits for an idle session, and never interrupts

A due tick is held while a turn is running and while anything the person queued is still waiting,
and goes when both are done. A prompt the person typed in the middle of a loop is not a tick of
it: it is not asked when the next tick is due, and its ending does not reset the clock.

**Why.** A schedule is a request to be asked again, not a licence to interrupt. The person is
still the one using this session.

`verified-by: bravebot_tui::state::a_tick_waits_for_the_turn_in_flight_and_for_what_is_queued`
`verified-by: bravebot_tui::state::a_prompt_typed_during_a_loop_is_not_a_tick_of_it`

<a id="LOOP-7"></a>
### LOOP-7: how fast a loop may go, and how slow

| The wait | Shortest | Longest |
|---|---|---|
| an interval the person gave | 5 seconds | 7 days |
| a delay a turn asked for | 1 minute | 1 hour |

A number outside the bounds becomes the nearer of the two, and a person who wrote one is told
what it became rather than left believing they are watching something ten times more closely than
they are.

**Why the two rows differ.** An interval the person typed is their own number, and `10s` means
ten seconds. It is bounded at all only because a loop with no gap is a way to spend a rate limit
rather than a way to watch something, and because the interface has to stay usable between ticks;
it is capped only at the age a loop cannot outlive anyway. A delay a *turn* asked for is the
planner's number rather than the person's, so it is held much more tightly at both ends: a person
started the loop and is entitled to see it do something, and a turn that wants longer than an hour
can say so in its answer, where somebody reads it, rather than by going quiet for a day.

`verified-by: bravebot_tui::loops::an_interval_the_person_gave_is_kept_down_to_the_floor`
`verified-by: bravebot_tui::loops::an_interval_faster_than_the_floor_is_raised_to_it_and_said_so`
`verified-by: bravebot_tui::loops::an_interval_longer_than_a_loop_may_live_is_capped`
`verified-by: bravebot_tui::loops::an_interval_within_the_bounds_is_reported_as_unadjusted`
`verified-by: bravebot_tui::loops::a_wait_a_turn_asked_for_is_held_to_the_bounds`
`verified-by: bravebot_agent::tools::a_wait_outside_the_bounds_is_reported_as_the_one_that_will_happen`

<a id="LOOP-8"></a>
### LOOP-8: where the person gave an interval, the driver keeps it and no turn can change it

A turn that asks for a different wait is answered by the interval that was typed. Only a loop
nobody gave an interval for asks the planner anything about timing.

`verified-by: bravebot_tui::loops::a_paced_loop_ignores_what_a_turn_asked_for`
`verified-by: bravebot_tui::loops::a_self_paced_loop_waits_as_long_as_the_turn_asked`

<a id="LOOP-9"></a>
### LOOP-9: a self-paced tick that says nothing is woken once more, then the loop ends

Twenty minutes later, once. A second tick that ends without saying when to run again ends the
loop.

**Why.** The first silence is a turn that forgot. The second is a loop nobody is running, and
waking it every twenty minutes for the rest of the session helps nobody. A tick that does say
when to run again restores the fallback, because the budget is for turns that stopped answering.

`verified-by: bravebot_tui::loops::a_self_paced_turn_that_says_nothing_is_woken_once_more_and_then_the_loop_ends`
`verified-by: bravebot_tui::loops::a_turn_that_says_when_to_wake_restores_the_fallback`

<a id="LOOP-10"></a>
### LOOP-10: a tick is told that it is one, and which kind of loop it is in

The turn carries which tick it is and whether the loop is self-paced, and both are said in the
planner's own preamble. A tick paced by the person is told the timing is theirs and that there is
no tool for it; a self-paced tick is told the loop lasts as long as it keeps saying when to run
again.

**Why.** The driver is the only thing that knows a turn is a tick. A planner that cannot tell
answers as though somebody had just typed the line for the first time, which is the failure a loop
exists to avoid, and one that half-knows goes looking for the scheduling tool an interval loop
never has and tells the user it is missing. Both were observed before this clause existed.

`verified-by: bravebot_agent::turn::a_tick_is_told_that_it_is_one_and_which_kind_of_loop_it_is_in`

## What ends one

<a id="LOOP-11"></a>
### LOOP-11: four things end a loop, and each of them says so

| What | When |
|---|---|
| the person interrupts | Ctrl-C, read against the loop after the turn in flight and the line in the box, and before leaving |
| a turn is stopped | any turn cancelled while a loop runs, whether or not it was a tick |
| the session moves on | `/clear`, and leaving |
| age | seven days after it started |

Ctrl-C reaches the loop before it reaches the session, so the key that stops a thing that keeps
happening is not also the key that ends everything. It reaches the half-written line first,
because that is nearer still.

`verified-by: bravebot_tui::app::interrupting_stops_the_loop_before_it_leaves`
`verified-by: bravebot_tui::app::interrupting_clears_the_line_before_it_stops_the_loop`
`verified-by: bravebot_tui::state::clearing_the_session_ends_the_loop`
`verified-by: bravebot_tui::state::stopping_a_loop_says_so_and_says_nothing_when_there_was_none`
`verified-by: bravebot_tui::loops::a_loop_older_than_a_week_has_aged_out`

<a id="LOOP-12"></a>
### LOOP-12: a loop is never written down

It is not in the session record, so it is not restored by a resume and does not survive the
process.

**Why.** A schedule that outlived the session that set it would start sending prompts at somebody
who opened a conversation to read it, with no visible cause and nothing in the transcript to
explain it. The gesture that starts a loop is the gesture that keeps it: while this session is
open.

`verified-by: by-construction (the loop is a private field of the interface's session state and is not among the fields written to a session record)`

<a id="LOOP-13"></a>
### LOOP-13: what is going to happen is on the screen

Each tick is announced with its number, and with how many ticks in a row have reported finding
nothing where there have been any. `/status` says what is repeating and when the next one is due;
a session with no loop says nothing about loops.

**Why.** The count of quiet ticks is the difference between a loop that is working and a loop
that has nothing to do, and without it a long watch is twenty identical answers nobody reads. And
what happens next without anybody typing anything is the one thing about a session that cannot be
read off the transcript.

`verified-by: bravebot_tui::status::the_report_says_what_is_repeating_and_when_it_is_next_due`
`verified-by: bravebot_tui::status::a_session_with_no_loop_does_not_mention_one`
`verified-by: bravebot_tui::loops::quiet_ticks_are_counted_until_one_reports_something`

## Known costs

- **A loop keeps spending.** Every tick is a turn, with the whole conversation re-sent, and
  nothing here bounds the total. The interval and the session's own life are the only limits, and
  a five-minute loop left open overnight is a hundred and fifty turns nobody read.
- **A self-paced loop is paced by the thing it is watching over.** The planner chooses the wait
  from a context it also wrote, so a turn that misjudges what it is waiting for will keep
  misjudging it. The bounds hold the cost to one turn an hour at worst; nothing holds the
  usefulness.
