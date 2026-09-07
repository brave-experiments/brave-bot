---
id: AGENT
title: spawn_agent
status: normative
governs:
  - crates/agent/src/tools.rs
---

## Scope

The call that starts a delegated agent. `kind` is routing; `task` and `each` are content. The
call answers as soon as the kernel has approved it, and the report follows later. What a delegate
is, what it may do and how long it may live is [delegation.md](../delegation.md); this spec is the
call surface.

## Clauses

<a id="AGENT-1"></a>
### AGENT-1: `kind` selects from a fixed set, and is the field a person could approve

It decides what the run holds, which makes it routing, and it is a name out of a list the driver
wrote rather than anything the call describes. That is what a person could approve on its own:
that a delegate may read, or read and run, or read and run and write. A name matching nothing in
the list is refused and reaches no capability set.

`verified-by: bravebot_core::policy::a_kind_nobody_enumerated_is_refused`
`verified-by: bravebot_core::delegate::a_kind_is_selected_from_the_enumerated_set_and_nothing_else`

<a id="AGENT-2"></a>
### AGENT-2: `task` is the whole of what the delegate is told, and it may not be private

It is the only thing steering the call, and it comes from a context holding nothing an attacker
wrote. The delegate cannot see the conversation it came from, so a task that leaves something out
is a delegate that never learns it.

`verified-by: bravebot_core::policy::a_private_task_cannot_direct_a_delegate`
`verified-by: bravebot_core::policy::a_run_that_has_met_something_untrusted_cannot_delegate`

<a id="AGENT-3"></a>
### AGENT-3: the result is that a delegate started, and the report arrives on its own later

The call answers with the delegate's number as soon as the kernel has approved one, and the
planner has its round back. What the delegate says arrives as a message of its own, before the
planner is next asked what to do.

Not as the result of this call. A result answers a call once, and by the time a delegate has
anything to say the call it came from was answered rounds earlier. A call that waited for the
report would hold the turn still until the delegate finished, so a turn could have one delegate
working and no more.

`verified-by: bravebot_agent::turn::two_delegates_work_at_the_same_time`
`verified-by: bravebot_agent::turn::a_delegates_report_reaches_the_planner_that_asked_for_it`

<a id="AGENT-4"></a>
### AGENT-4: what shape the report takes is not the tool's to decide

The delegate's answer is still labelled when it arrives, and presented like any other result.
The tool reads none of it: whether the planner is shown the words or a reference to them follows
from the label the delegate's own context earned.

`verified-by: bravebot_agent::turn::a_delegates_report_reaches_the_planner_that_asked_for_it`
`verified-by: bravebot_agent::turn::what_a_delegate_read_never_reaches_the_planner_that_asked`

<a id="AGENT-5"></a>
### AGENT-5: one call may fan a task out, and every delegate it starts is gated on its own

`each` names one delegate per entry. Each is told the shared `task` followed by its own entry,
and is otherwise a delegate like any other: it passes the same gate, takes its own number, and
holds its own copy of what a person has vouched for. A call naming more than eight, or naming
none, is refused and starts nothing.

**Why.** A planner starting four delegates one call at a time writes four near-identical
paragraphs, and none of them starts until the last word of the last copy is written. The prose is
on the critical path and only the driver reads it.

**Why each is gated separately.** A fan-out is several runs. A gate that saw one of them would be
approving the rest on the strength of a sibling, and the entries are the part that differs.

**Why a ceiling at all.** Not authority: the planner can already start any number one call at a
time, and each passes the same gate either way. It is the cost of asking that changes, and a
field turning one sentence into an unbounded number of runs is worth a bound.

`verified-by: bravebot_agent::turn::one_call_can_fan_a_task_out_over_several_delegates`
`verified-by: bravebot_agent::turn::a_fan_out_past_the_ceiling_is_refused_and_starts_nothing`
