---
id: TURN
title: What bounds a turn
status: normative
governs:
  - crates/agent/src/turn.rs
---

## Scope

How long a turn may go on, and what happens when it does not stop.

## Clauses

<a id="TURN-1"></a>
### TURN-1: a turn may make 40 rounds, and then loses its tools rather than ending

On the fortieth round the next request offers no tools, the planner is told it has none left, and
it answers with what it has. A call it asks for anyway is dropped rather than run.

**Not a safety property.** A gate refuses on the thousandth round what it refuses on the first.
This is a bound on futility: in a directory nobody vouched for, a planner looking for a file it
cannot name will try glob after glob for as long as anyone lets it. Do not cite this clause as a
containment measure.

`verified-by: bravebot_agent::turn::a_turn_that_keeps_calling_tools_is_made_to_answer`
`verified-by: bravebot_agent::turn::calls_made_after_the_budget_is_spent_are_not_run`
`verified-by: bravebot_agent::turn::a_turn_is_not_cut_off_after_a_fixed_number_of_rounds`
