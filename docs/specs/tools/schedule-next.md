---
id: SCHED
title: schedule_next
status: normative
governs:
  - crates/agent/src/tools.rs
---

## Scope

Saying when a self-paced loop should run again. `delay_seconds` and `noop` are routing;
`reason` is content. The result is a confirmation naming the wait that will actually happen.

What a loop is, where its prompt comes from, and what ends one is [loop.md](../loop.md).

## Clauses

<a id="SCHED-1"></a>
### SCHED-1: the only thing this decides is a moment

There is no argument for what the next run asks. The prompt is the line the person typed when
they started the loop, held by the interface, and it is sent again unchanged.

**Why.** This is what makes the routing approvable on its own. "Ask me that again in twenty
minutes" can be read and agreed to without knowing what "that" is; a field naming the next prompt
would make the same call unreadable, and would let a turn write its own next instruction.

`verified-by: bravebot_agent::tools::nothing_on_this_tool_says_what_the_next_turn_asks`

<a id="SCHED-2"></a>
### SCHED-2: it is offered to a tick of a self-paced loop and to nothing else

Every other turn is offered no such tool, and a call to it from one is answered the way any other
name nobody offered is: no such tool.

**Why.** A tool that quietly worked where it was not offered would let a turn that nobody is
looping schedule itself, and a tool that is present but inert is one the planner has to be told
to ignore.

`verified-by: bravebot_agent::tools::the_tool_that_paces_a_loop_is_offered_only_to_a_tick_of_one`
`verified-by: bravebot_agent::turn::a_tick_of_a_self_paced_loop_says_when_to_run_again`
`verified-by: bravebot_agent::turn::a_turn_that_is_not_a_tick_cannot_schedule_one`

<a id="SCHED-3"></a>
### SCHED-3: the wait is held to its bounds before it is reported back

Between a minute and an hour. The number the planner is told is the number it is getting, not the
number it asked for.

**Why.** A tool that echoed what it was given would have the next answer describing a schedule
that is not going to happen.

`verified-by: bravebot_agent::tools::a_wait_outside_the_bounds_is_reported_as_the_one_that_will_happen`

<a id="SCHED-4"></a>
### SCHED-4: a call missing a delay or a verdict is refused rather than filled in

`delay_seconds` and `noop` are both required, and nothing is scheduled without them.

**Why.** Whether a tick found anything is what the count of quiet ticks is built from, so a turn
that leaves it out is asking for a number to be invented on its behalf and shown to somebody as
an observation.

`verified-by: bravebot_agent::tools::a_schedule_missing_what_it_needs_is_refused`

<a id="SCHED-5"></a>
### SCHED-5: what the turn says it is waiting on reaches a screen and stops there

`reason` is the planner's own words, carried at the integrity of the context they came from and
released for display like any other line a tool puts on the screen. Nothing waits on it, no later
turn reads it back, and it is not sent anywhere.

`verified-by: bravebot_agent::tools::what_the_turn_is_waiting_on_reaches_the_person_watching`
