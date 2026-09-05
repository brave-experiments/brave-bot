---
id: DELEGATE
title: Delegated agents
status: normative
governs:
  - crates/core/src/delegate.rs
  - crates/core/src/policy.rs
  - crates/agent/src/delegate.rs
guards:
  - symbol: Policy::before_delegate
  - symbol: Policy::adopt_from_delegate
  - symbol: Policy::sink
  - symbol: Task::delegated
---

## Scope

A second planner, spawned by the first, with its own context and a narrower set of capabilities.
What fixes it before it exists, what it may do, what crosses back, and what does not. What a
processor is, and why that one holds nothing at all, is [processors.md](processors.md).

## Why it exists

A planner that runs the build reads the whole log. A planner that asks a delegate to run the
build is told what failed. The work happens either way and only one of them spends the
conversation on it, so a long task in a large tree stops being a race between finishing and
filling the context.

Nothing here is about trusting a second model more than the first. It is about where the reading
lands.

## What a delegate is

<a id="DELEGATE-1"></a>
### DELEGATE-1: a delegate is a planner, not a processor with tools

| | |
|---|---|
| Tools | its kind's, which are what its capabilities reach, and never a way to delegate |
| Memory | none of its parent's exchange: it begins with the task it was given |
| Conversation | a loop of its own, bounded |
| Reads | whatever its capabilities and the paths a person vouched for allow |
| Writes | files, each shown to a person first, and slots in a quarantine of its own |

A processor holds nothing at all because a processor with one tool would be a second planner with
untrusted content in its context. A delegate is the other half of that sentence: it holds
capabilities and it holds no untrusted content. Content it may not read is quarantined and it is
handed a reference, exactly as its parent would be, so there is no point in the run where
untrusted bytes and a capability are in the same context.

`verified-by: bravebot_agent::tools::a_delegate_is_offered_only_the_tools_its_capabilities_reach`
`verified-by: bravebot_agent::turn::what_a_delegate_could_not_read_is_quarantined_from_it_too`

<a id="DELEGATE-2"></a>
### DELEGATE-2: a run that has met something untrusted cannot delegate

The task is composed by the run that spawns the delegate and is the whole of what steers it, so a
run whose own context has met untrusted bytes composes tasks that are a function of them.
Delegating from one is refused outright rather than narrowed.

Private content is refused too, on either the task or the kind: the user's data may not become
another planner's prompt.

**Why.** This is the clause the rest rests on. Without it, delegation is a way for untrusted
content to reach a planner's context by the long way round, and every other rule here would be
describing something that had already failed.

`verified-by: bravebot_core::policy::a_run_that_has_met_something_untrusted_cannot_delegate`
`verified-by: bravebot_core::policy::a_private_task_cannot_direct_a_delegate`

## What is fixed before it runs

<a id="DELEGATE-3"></a>
### DELEGATE-3: a kind is selected from a set the driver enumerated

The planner names a kind and cannot describe one. A name is compared against a fixed list and
matches or does not; a name matching nothing is refused, and there is no spelling of it that
reaches a capability set nobody wrote down.

`verified-by: bravebot_core::delegate::a_kind_is_selected_from_the_enumerated_set_and_nothing_else`
`verified-by: bravebot_core::delegate::every_advertised_name_resolves_to_the_kind_it_names`
`verified-by: bravebot_core::policy::a_kind_nobody_enumerated_is_refused`

<a id="DELEGATE-4"></a>
### DELEGATE-4: a delegate holds its kind's capabilities, narrowed by its parent's

The intersection, computed before the delegate exists and never widened afterwards. Delegation
redistributes authority and never creates it, so a delegate can hold nothing the run that spawned
it did not already hold, and a kind asking for more gets a delegate without it.

The three kinds are ordered, so choosing a wider one never costs a narrower one's reach:

| Kind | Holds | For |
|---|---|---|
| `reader` | reading | finding something out |
| `checker` | reading, running programs | finding out whether something works |
| `worker` | reading, running programs, writing files | finishing a sub-task |

Every kind additionally reaches the network, because a planner is a model call and the request out
is egress like any other. No tool a delegate is offered reaches it, so what it buys is the ability
to ask the endpoint on that delegate's behalf. A kind without it is a kind that cannot think.

`verified-by: bravebot_core::policy::a_delegate_holds_no_more_than_the_run_that_spawned_it`
`verified-by: bravebot_core::delegate::the_kinds_are_ordered_by_what_they_hold`
`verified-by: bravebot_core::delegate::every_kind_can_reach_the_endpoint_and_nothing_else_remote`

<a id="DELEGATE-5"></a>
### DELEGATE-5: the prompt belongs to the kind, and the planner writes no word of it

What a delegate is told about itself is a constant chosen by its kind. The planner supplies the
task and nothing else, so there is no sentence it can write that changes what a delegate is
rather than what it is doing.

`verified-by: bravebot_agent::delegate::each_kind_is_told_what_it_cannot_do`
`verified-by: bravebot_agent::delegate::every_kind_is_told_the_guidance_the_planner_is_told`

<a id="DELEGATE-6"></a>
### DELEGATE-6: a delegate is bounded, and the bound is its kind's

Every kind carries a round limit and the call cannot set one. On the limiting round the delegate
loses its tools rather than its run, and answers with what it has.

**Not a safety property.** A gate refuses on the last round what it refuses on the first. It
bounds futility, and it applies here because the person who would otherwise be the bound is
watching the turn, and the turn is blocked.

`verified-by: bravebot_core::delegate::every_kind_carries_a_bound`
`verified-by: bravebot_core::policy::a_delegates_bound_comes_from_its_kind`

<a id="DELEGATE-7"></a>
### DELEGATE-7: a delegate cannot delegate

No kind is offered the tool, and a call to it from inside a delegate is answered the way any
other unknown name is. Two refusals rather than one, because the depth is what bounds the whole
tree and a bound resting on the tool list alone rests on the model reading it.

**Why.** The bound on a tree of delegates is the product of the bounds, which is a number nobody
chose. And a person approving a write at the third level has no way to see which task it belongs
to.

`verified-by: bravebot_agent::tools::a_delegate_is_never_offered_a_way_to_delegate`
`verified-by: bravebot_agent::turn::a_call_to_spawn_agent_from_inside_a_delegate_does_nothing`

## What comes back

<a id="DELEGATE-8"></a>
### DELEGATE-8: the report is model output, labelled by the context that produced it

A delegate's answer is labelled at the integrity of its own context and presented to its parent
through the same gate every other result passes. Ordinarily that context has met nothing
untrusted and the words are shown; where it has, the answer is quarantined and the parent is
handed a reference to it.

Nothing is relabelled, and nothing is trusted on a delegate's say-so. The label is the one its
own context earned.

`verified-by: bravebot_agent::turn::a_delegates_report_reaches_the_planner_that_asked_for_it`

<a id="DELEGATE-9"></a>
### DELEGATE-9: nothing but the report crosses back

The exchange, the tool results, the narration and the quarantine all end with the delegate. A
reference minted inside one names nothing afterwards, and none of it can be asked for later.

**Why.** This is the feature rather than a restriction on it. A delegate whose reading reached
its parent's context would have moved the log rather than absorbed it.

`verified-by: bravebot_agent::turn::what_a_delegate_read_never_reaches_the_planner_that_asked`

<a id="DELEGATE-10"></a>
### DELEGATE-10: a delegate's effects are gated on their own

Every write and every run passes the same gates with its own single-use endorsement, so a person
sees the path and the diff whoever proposed them. An approval given inside a delegate cannot be
replayed by its parent, and one the parent already holds does not carry in.

Delegation saves context. It never saves an approval.

`verified-by: bravebot_agent::turn::a_delegates_write_is_approved_on_its_own`

<a id="DELEGATE-11"></a>
### DELEGATE-11: what a person vouched for outlives the delegate

The paths and the commands a person answered about come back to the session. They are standing
decisions about their own machine, and the record of them belongs to the session rather than to
whichever run happened to be going when they made it: a delegate told once that the build may run
must not leave the next one asking again.

Nothing else about a delegate's policy survives it.

`verified-by: bravebot_core::policy::what_a_person_vouched_for_inside_a_delegate_is_kept`

<a id="DELEGATE-12"></a>
### DELEGATE-12: a delegate puts no question of its own to a person

It is offered no way to ask one and no task list to write to. Its task came from a planner rather
than from the person, so a question about it asks somebody to arbitrate something they never set
up, and the list on the screen belongs to the turn they are actually watching.

What it could not settle goes in the report, and the parent asks.

`verified-by: bravebot_agent::tools::a_delegate_is_offered_no_task_list_and_no_way_to_ask`
`verified-by: bravebot_agent::delegate::no_kind_is_told_it_may_ask_a_person_or_delegate`

<a id="DELEGATE-13"></a>
### DELEGATE-13: one trail records both runs

A delegate's gates report into the same audit trail as the turn that spawned it, named so the two
can be told apart. A nested run recording somewhere else would leave a hole in the record exactly
over the part of the turn nobody watched.

`verified-by: bravebot_agent::turn::one_trail_records_the_delegate_and_the_turn_that_spawned_it`
`verified-by: bravebot_core::delegate::a_description_names_what_it_holds_but_never_the_task`

## Known costs

- **A reference cannot be handed to a delegate.** A parent working in a directory nobody vouched
  for holds references and no filenames, and there is no argument for passing one on: quarantines
  do not cross, so a name from one would resolve to nothing or, worse, to something else. A
  delegate has to list and read its way to the same file itself, which costs a round and the
  tokens of a listing. Sharing one quarantine between the two would fix that and would put a
  second place for every reference to resolve, which is the more expensive mistake.

- **A person approving a write cannot see which delegate asked.** The confirmation shows the path
  and the diff, as it always does, and the delegate's own lines are on the screen above it, but
  the prompt itself does not say that a delegate rather than the turn is asking. Where a turn
  spawns delegates one after another, a person reading only the prompt is approving a change
  whose reason is a task they did not read.

- **A delegate's task is a guess about what it will need.** It cannot come back for more and it
  cannot ask, so a task missing a detail is a delegate that reports having been unable to finish,
  and the round it spent is spent. The alternative is a channel back to the planner, which is a
  conversation, and a conversation is the context this exists to avoid.
