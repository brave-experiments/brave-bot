---
id: BACKEND
title: Backends
status: normative
governs:
  - crates/agent/src/backend.rs
  - crates/config/src/bedrock.rs
  - crates/config/src/settings.rs
---

## Scope

Where a request for a reply goes. Two services can answer: the aichat endpoint Brave runs, and
Claude on AWS Bedrock through somebody's own account. This file governs which of them serves a
given request, what a person is offered to choose from, and what a configuration may decide.

The wire protocol of either service is ordinary code. So is signing, which
[network-egress.md](network-egress.md) covers as the one way out. What a reply is labelled once it
arrives is [labels.md](labels.md).

## Clauses

<a id="BACKEND-1"></a>
### BACKEND-1: a settings file may name a destination and never a permission

What a person's settings may say is which region, which credential profile, and which model each
tier names. Nothing in that file grants a capability, vouches for a path, or decides whether an
effect is allowed.

The block does not become the process environment either. A value is consulted where a variable
would be, and reaches a subprocess only where that subprocess is the thing it configures.

**Why.** The file is read before anything runs and is the easiest thing on the machine to write to,
so a permission that could be granted from it would be a permission granted by whatever last edited
it. Installing the names globally would put every one of them in front of every command the agent
ever starts, which is a far larger claim than "this is how I reach the backend".

`verified-by: by-construction (the block is read as a flat map of strings and only ever consulted by name; nothing exports it, and the sole value handed to a subprocess is the profile, passed as an argument to the tool that owns it)`

<a id="BACKEND-2"></a>
### BACKEND-2: configuring a second backend takes nothing away from the first

A settings block naming AWS tiers does not change which model answers when nobody has chosen one,
and does not change how large a request may get before the conversation is shortened.

**Why.** Every build can reach Brave, and that is what somebody has before they configure anything.
Adding a way to reach more models should not quietly move the default onto one of them, nor set a
budget from a window that belongs to a model the session may never use.

`verified-by: bravebot_config::lib::a_bedrock_block_does_not_change_the_default_model`
`verified-by: bravebot_config::lib::a_bedrock_block_does_not_move_the_budget_off_the_default`

<a id="BACKEND-3"></a>
### BACKEND-3: the model names the service, and nothing else selects one

A request goes to whichever service offers the model it names. No other fact participates: not
which configuration is present, not which service answered last, not which one a person used
first.

**Why.** Where both are reachable a configuration cannot say where a request belongs. Bedrock
refuses a model it does not recognise rather than substituting one, and the aichat endpoint has
never heard of an inference-profile ARN, so a request sent on the strength of anything but the name
fails at the far end for a reason nothing local could explain.

**Note.** The name is not content. It comes from a configured default or from a person picking off a
list they read, and a model's own output never reaches it.

`verified-by: bravebot_agent::backend::a_configured_bedrock_model_selects_the_bedrock_backend`
`verified-by: bravebot_agent::backend::a_brave_model_still_reaches_aichat_while_bedrock_is_configured`
`verified-by: bravebot_agent::backend::without_bedrock_configured_the_aichat_backend_is_selected`

## Known costs

- **A credential is resolved by running the AWS CLI.** Reaching Bedrock needs short-lived keys that
  expire during a session, and the tool that holds them is the one the person already signs in
  with. That is a process this code did not write, reading a configuration this code does not
  govern.

- **The assumed AWS window is a guess.** No endpoint there reports a context window, and an
  inference-profile ARN does not say which model it resolves to, so one figure stands in for every
  tier: the one an unresolvable profile actually gets. It is deliberately low, because being wrong
  upward removes shortening rather than delaying it.
