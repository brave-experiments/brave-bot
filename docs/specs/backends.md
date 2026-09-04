---
id: BACKEND
title: Backends
status: normative
governs:
  - crates/agent/src/backend.rs
  - crates/bedrock/src/credentials.rs
  - crates/tui/src/app.rs
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

What a person's settings may say is which region, which credential profile, which model each tier
names, and which model to request when nobody has chosen one. Nothing in that file grants a
capability, vouches for a path, or decides whether an effect is allowed.

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

<a id="BACKEND-4"></a>
### BACKEND-4: what a person may choose is every model any reachable service offers

Configuring a second backend puts its models on offer beside the first one's rather than in place of
them.

**Why.** A roster that replaced the other left somebody who named a single tier with a picker
offering exactly one model and no way back to the ones every build has. Reaching more models is not
a reason to stop reaching the existing ones.

`verified-by: bravebot_tui::app::configured_tiers_are_offered_alongside_the_brave_roster`

<a id="BACKEND-5"></a>
### BACKEND-5: only models that can actually be reached are offered

A tier appears when the configuration names a model for it, and not otherwise. A service's own
roster is offered only where this build holds the credentials to reach it. Nothing is invented for a
tier that was left unset.

**Why.** A name on the list is a promise that picking it works. An ARN cannot be derived from a
model name, so an entry guessed for an unnamed tier is a choice that fails remotely, and a build
pointed only at AWS has no Brave credentials, so offering that roster would list models whose every
request fails unsigned.

`verified-by: bravebot_tui::app::a_tier_with_no_model_configured_is_not_offered`
`verified-by: bravebot_config::lib::without_brave_credentials_the_default_is_the_strongest_bedrock_tier`

<a id="BACKEND-6"></a>
### BACKEND-6: a row says which service will answer it

Where the same model is reachable through more than one service, what a person reads says which one
a given row is, in terms that cannot collide with a name a service chose for itself.

**Why.** The two are billed differently and authenticate differently, so which one answers is the
whole of what is being chosen between. Naming the service is not enough: Brave serves part of its own
roster through Bedrock and says so in the names it sends, so that word appeared on both halves of the
list and distinguished nothing.

`verified-by: bravebot_tui::app::a_configured_tier_is_not_confusable_with_a_brave_model_served_through_bedrock`
`verified-by: bravebot_tui::app::a_tier_with_no_profile_configured_still_names_the_account`

<a id="BACKEND-7"></a>
### BACKEND-7: the conversation budget belongs to the model in force

How large a request may get before the conversation is shortened is taken from the model that will
answer it, at the moment that model is chosen.

**Why.** A budget above the real window does not shorten a conversation late, it stops shortening it
at all, silently: every round asks, no round qualifies, and the session runs to exhaustion looking
like one with nothing to summarise.

`verified-by: bravebot_tui::app::a_bedrock_entry_carries_the_window_the_budget_is_taken_from`
`verified-by: bravebot_tui::app::the_window_of_a_model_chosen_earlier_is_found_in_the_listing`

<a id="BACKEND-8"></a>
### BACKEND-8: an unreachable listing costs only what it described

One service failing to say what it offers does not withdraw models known from configuration alone. A
choice is refused only when there is nothing left that could be chosen.

**Why.** Configured tiers need no network to know. Refusing the whole picker because one half was
unreachable would leave the only models this configuration can definitely reach unpickable, which is
the position somebody offline is most likely to be in.

`verified-by: bravebot_tui::app::an_unreachable_listing_still_offers_the_configured_tiers`
`verified-by: bravebot_tui::app::an_unreachable_listing_with_no_tiers_configured_is_still_a_failure`

<a id="BACKEND-9"></a>
### BACKEND-9: a sign-in is asked for before work starts, by the model about to answer

Where a service authenticates interactively and has no usable session, the sign-in happens before a
request is attempted, and only for the service the next request will actually go to. What it asks of
the person is shown where they are already reading, line by line as it is written, and the interface
keeps its display throughout.

**Why.** A sign-in prints a URL and a code and then waits for them to be used, so those lines are the
flow rather than a report of it: shown after the fact, or collected and printed at the end, they
arrive once the code has stopped working. Giving the screen away instead puts them under a display
that is about to redraw over them, and leaves somebody in a terminal that no longer resembles the
program they were using. Doing it up front is what keeps it off the request path, where the work has
begun and nobody is being asked anything. Asking by the model rather than by what is configured
matters because otherwise a turn served entirely by one backend stops to authenticate against
another it will never call.

`verified-by: bravebot_agent::backend::a_brave_model_never_needs_an_aws_sign_in`
`verified-by: bravebot_agent::backend::without_bedrock_configured_nothing_needs_a_sign_in`
`verified-by: bravebot_agent::backend::signing_in_for_a_model_no_aws_account_serves_does_nothing`

<a id="BACKEND-10"></a>
### BACKEND-10: asking whether a session is good costs nothing once it is known to be

Establishing that a service has a usable session runs its tool once. Until the credential that
answer came from is close enough to its own stated expiry to be no use to the request that follows,
the same question is answered without running anything. A session that is not good is never reported
as one, and an answer with no stated expiry is not kept.

**Why.** The check happens before every turn, and the tool that answers it takes most of a second, so
paid each time it is a pause between pressing Enter and seeing the line appear. The expiry is the
credential's own word about how long the answer stays true, which is why it and not a fixed interval
is what bounds this. Stopping short of it matters because the answer is used to decide whether to
sign in before work that then has to be signed: taken at the last second, the request that follows
carries a credential that has already expired.

`verified-by: bravebot_bedrock::credentials::a_session_already_shown_to_be_good_is_not_asked_about_again`
`verified-by: bravebot_bedrock::credentials::a_session_that_has_run_out_is_asked_about_again`
`verified-by: bravebot_bedrock::credentials::a_session_about_to_run_out_is_treated_as_already_gone`
`verified-by: bravebot_bedrock::credentials::one_profile_being_good_says_nothing_about_another`
`verified-by: bravebot_bedrock::credentials::the_default_profile_is_remembered_like_any_other`
`verified-by: bravebot_bedrock::credentials::a_session_with_no_stated_expiry_is_not_kept`
`verified-by: bravebot_bedrock::credentials::an_expiry_is_converted_to_the_instant_it_names`
`verified-by: bravebot_bedrock::credentials::the_expiry_the_cli_reports_is_read_from_the_process_format`
`verified-by: bravebot_bedrock::credentials::an_expiry_that_is_not_the_expected_shape_is_not_guessed_at`

<a id="BACKEND-11"></a>
### BACKEND-11: a settings file names the model above what the build baked in

Where a settings file names a model and an exported variable does not, that name is what a request
uses, in preference to the model compiled into the binary. A choice already recorded with `/model`
still wins over all of it.

**Why.** Every release bakes a default model in, so this value ranked like the rest of the file would
lose on every binary anybody was given: the key would parse, `doctor` would report it, and nothing
would change outside a source build. An exported variable stays above the file because it is the most
specific thing a person said, and a recorded pick stays above both because it is the more recent one.

`verified-by: bravebot_config::lib::a_model_in_the_settings_file_outranks_the_baked_in_one`
`verified-by: bravebot_config::lib::an_exported_model_outranks_the_settings_file`
`verified-by: bravebot_config::lib::the_env_block_spelling_stays_below_the_baked_in_value`

<a id="BACKEND-12"></a>
### BACKEND-12: a tier word names a model some reachable service serves

`opus`, `sonnet` and `haiku` name a tier rather than a model. Each resolves to a model that can
actually be reached: the model an AWS account named for that tier, and otherwise that tier's name on
the roster every build can reach. A tier word is never sent as the bare word. Any other name is used
as written.

**Why.** Those three words are what a settings file written for another tool puts in this key, so
they are the common case rather than an edge one. Sent unresolved they reach a service that has never
heard of them: Bedrock refuses an unknown model, and the aichat endpoint silently resets one, which
makes the key appear to work while changing nothing. An AWS account that named the tier wins because
naming it is asking for it, and a tier it left unset falls through rather than being guessed at,
since an ARN cannot be derived from a word. The exception is a build holding no Brave credentials,
where a Brave name reaches a service it cannot sign for.

**Note.** The Brave names are compiled in rather than matched against the model listing. A
configuration is built without touching the network, and a one-shot run never asks for that listing:
only the interactive picker does. Resolving a word against it would put a round trip in front of every
one-shot run to expand one word, and would fail with no network where it currently succeeds. The cost
is that the service owns those names, and a renamed one is reset by the endpoint to `automatic`, which
is where somebody with no `model` key already starts.

`verified-by: bravebot_config::lib::a_tier_alias_resolves_to_the_model_that_tier_names`
`verified-by: bravebot_config::lib::a_tier_alias_without_bedrock_resolves_against_the_brave_roster`
`verified-by: bravebot_config::lib::an_alias_for_an_unconfigured_tier_falls_through_to_brave`
`verified-by: bravebot_config::lib::without_brave_credentials_an_unconfigured_tier_stays_on_aws`
`verified-by: bravebot_config::lib::a_model_that_is_not_a_tier_alias_is_used_as_written`
`verified-by: bravebot_config::bedrock::every_tier_names_a_brave_model`
`verified-by: bravebot_config::bedrock::the_tiers_name_different_brave_models`

## Known costs

- **A credential is resolved by running the AWS CLI.** Reaching Bedrock needs short-lived keys that
  expire during a session, and the tool that holds them is the one the person already signs in
  with. That is a process this code did not write, reading a configuration this code does not
  govern.

- **The assumed AWS window is a guess.** No endpoint there reports a context window, and an
  inference-profile ARN does not say which model it resolves to, so one figure stands in for every
  tier: the one an unresolvable profile actually gets. It is deliberately low, because being wrong
  upward removes shortening rather than delaying it.
