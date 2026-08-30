---
id: ROUTE
title: Routing and content
status: normative
governs:
  - crates/core/src/capability.rs
  - crates/core/src/policy.rs
guards:
  - symbol: Policy::before_granted_action
  - symbol: Policy::promote_confined_read
  - symbol: Policy::path_of_reference
---

## Scope

An **effect** is anything the agent does that changes something or leaves this process: a file
written, a program run, a request sent over the network, a question put on somebody's screen. A
read is not an effect, because it changes nothing and stays inside the working directory. That
asymmetry runs through everything below: the planner may choose what to read, and never what an
effect touches.

This spec is where an effect is allowed to land and what may decide it. What a label means is in
[labels.md](labels.md). Which paths a person vouched for is in [trust-map.md](trust-map.md).

## The split

<a id="ROUTE-1"></a>
### ROUTE-1: every effect splits into routing and content

**Routing** decides where an action goes: a file path, a URL, a recipient, an argument vector.
**Content** is the payload that is merely carried. No field is both, and nothing at run time
reclassifies one. Which argument is which is fixed per tool by the tool surface.

**Why.** This asymmetry is half the mechanism. Untrusted text can be carried into an action as
content but can never become routing, so it cannot redirect anything. The other half is that it
never reaches a component that decides, neither the planner nor the driver.

`verified-by: bravebot_core::policy::fetched_content_can_be_written_but_cannot_choose_the_path`

<a id="ROUTE-2"></a>
### ROUTE-2: routing must be `(T,pub)`

Derived only from trusted input, and never from fetched content. Untrusted routing is an injection
attempt and is refused. Trusted-but-private is refused too, since a routing field ends up
somewhere this policy stops governing.

`verified-by: bravebot_core::policy::routing_refuses_untrusted_values`
`verified-by: bravebot_core::policy::routing_refuses_private_values`
`verified-by: bravebot_core::value::trusted_private_values_are_not_routing_safe`

<a id="ROUTE-3"></a>
### ROUTE-3: content may be untrusted, but must not be private at release

Carrying bytes decides nothing, so untrusted content is ordinary. Private content is different:
releasing it hands the user's data somewhere this policy no longer governs. Writing back into the
workspace is the one move that lowers confidentiality without releasing anything, because the
destination is inside the boundary the bytes came from.

`verified-by: bravebot_core::policy::a_write_back_into_the_workspace_lowers_only_confidentiality`
`verified-by: bravebot_core::policy::private_input_asks_even_for_a_vouched_command`

## The one relaxation

<a id="ROUTE-4"></a>
### ROUTE-4: a read path may be promoted, and nothing else may

Promotion lets the model choose which file to read next, because a read
changes nothing and is confined to the workspace. Every such choice is recorded as a promotion, so
an audit separates the model's decisions from the user's.

It **must never be used for an effect**. Private content cannot be promoted, and content cannot be
promoted by being read aloud: what is promoted is the model's own proposal, not bytes that came
back from somewhere.

`verified-by: bravebot_core::policy::a_model_proposal_can_be_promoted_for_a_confined_read`
`verified-by: bravebot_core::policy::private_content_cannot_be_promoted`
`verified-by: bravebot_core::policy::a_file_cannot_be_promoted_by_reading_it_aloud`

## Effects

<a id="ROUTE-5"></a>
### ROUTE-5: an effect needs a capability and a human endorsement

An action whose capability was not granted is refused outright. A person's
approval mints a **single-use** endorsement bound to that exact value, so it cannot be replayed,
redirected to another value, or reused for a different pipeline.

A write that would make a trusted path untrusted is the case that must be shown. Where nobody can
be asked, effects are refused rather than applied unseen.

`verified-by: bravebot_core::policy::a_granted_action_needs_a_matching_endorsement`
`verified-by: bravebot_core::policy::an_endorsement_cannot_be_replayed`
`verified-by: bravebot_core::policy::an_endorsement_does_not_transfer_to_another_value`
`verified-by: bravebot_core::policy::an_endorsement_does_not_authorise_a_different_pipeline`
`verified-by: bravebot_core::policy::ungranted_capabilities_are_refused`
`verified-by: bravebot_core::policy::a_run_needs_the_capability_as_well_as_the_endorsement`

<a id="ROUTE-6"></a>
### ROUTE-6: a reference is not a destination unless it names a file

Resolving a reference to the name it stands for is the only way that name leaves the policy layer,
and it
authorises nothing by itself. For a read the name is promoted exactly as ROUTE-4 promotes the model's own
choice. For a write it is never promoted: the name goes to a person, and the grant is issued for
the path they saw, which is why such a write always asks. A reference naming no file,
which is everything a processor produced, is refused as a destination.

**Why.** That refusal is what stops untrusted text choosing where an effect lands.

`verified-by: bravebot_core::reference::content_is_not_offered_as_a_destination`
`verified-by: bravebot_core::policy::a_reference_that_names_no_file_is_not_a_destination`
`verified-by: bravebot_agent::turn::a_processors_output_cannot_be_a_destination`

<a id="ROUTE-7"></a>
### ROUTE-7: before adding a tool, ask what its routing field is

If a person could not approve that field alone, the tool does not get built. A shell string is
destination and payload at once, which is why the planner has no shell and why `apply_patch` is
excluded. An argv vector passes the test, which is why running a pipeline of argv stages does not.

`verified-by: none`
