---
id: AGENT
title: spawn_agent
status: normative
governs:
  - crates/agent/src/tools.rs
---

## Scope

The call that starts a delegated agent. `kind` is routing; `task` is content. The call answers as
soon as the kernel has approved it, and the report follows later. What a delegate is, what it may
do and how long it may live is [delegation.md](../delegation.md); this spec is the call surface.

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
anything to say the call it came from was answered rounds ago. A call that waited instead would
mean a turn could only ever have one delegate working, which is the thing being removed.

`verified-by: bravebot_agent::turn::two_delegates_work_at_the_same_time`
`verified-by: bravebot_agent::turn::a_delegates_report_reaches_the_planner_that_asked_for_it`

<a id="AGENT-4"></a>
### AGENT-4: what shape the report takes is not the tool's to decide

The delegate's answer is still labelled when it arrives, and presented like any other result.
The tool reads none of it: whether the planner is shown the words or a reference to them follows
from the label the delegate's own context earned.

`verified-by: bravebot_agent::turn::a_delegates_report_reaches_the_planner_that_asked_for_it`
`verified-by: bravebot_agent::turn::what_a_delegate_read_never_reaches_the_planner_that_asked`
