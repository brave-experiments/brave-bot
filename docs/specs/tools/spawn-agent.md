---
id: AGENT
title: spawn_agent
status: normative
governs:
  - crates/agent/src/tools.rs
---

## Scope

The call that starts a delegated agent. `kind` is routing; `task` is content. The result is one
report. What a delegate is and what it may do is [delegation.md](../delegation.md); this spec is
the call surface.

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
### AGENT-3: the result is the report, and what shape it takes is not the tool's to decide

What comes back is the delegate's answer, still labelled, presented like any other tool result.
The tool reads none of it: whether the planner is shown the words or a reference to them follows
from the label the delegate's own context earned.

`verified-by: bravebot_agent::turn::a_delegates_report_reaches_the_planner_that_asked_for_it`
`verified-by: bravebot_agent::turn::what_a_delegate_read_never_reaches_the_planner_that_asked`
