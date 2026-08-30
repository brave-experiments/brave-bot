---
id: SPAWN
title: spawn_processor
status: normative
governs:
  - crates/agent/src/processor.rs
---

## Scope

The call that starts a processor. `reads` is routing; `instruction` is content. The result is a
reference. What a processor is and what it may do is [processors.md](../processors.md); this spec
is the call surface.

## Clauses

<a id="SPAWN-1"></a>
### SPAWN-1: the call names the slots it may read, and gets nothing else

A processor is given exactly the references named in `reads`. A reference naming nothing is
refused, a call with nothing to read is refused, and naming the same reference twice is refused.

`verified-by: bravebot_core::policy::only_the_slots_it_was_given_reach_a_processor`
`verified-by: bravebot_core::policy::a_reference_to_nothing_is_refused`
`verified-by: bravebot_core::policy::a_processor_with_nothing_to_read_is_refused`
`verified-by: bravebot_core::policy::naming_the_same_reference_twice_is_refused`

<a id="SPAWN-2"></a>
### SPAWN-2: the instruction comes from the planner and may not be private

It is the only thing steering the call, and it comes from a context holding nothing an attacker
wrote.

`verified-by: bravebot_core::policy::a_private_instruction_cannot_direct_a_processor`

<a id="SPAWN-3"></a>
### SPAWN-3: the result is a reference, never text

The planner is told the shape of what was produced and no more, so an instruction can ask the
processor to decide as well as to rewrite without any of that judgement reaching the planner.

`verified-by: bravebot_agent::turn::the_planner_is_told_the_shape_of_what_a_processor_produced`
`verified-by: bravebot_agent::turn::a_quarantined_file_is_rewritten_by_a_processor`
