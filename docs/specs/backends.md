---
id: BACKEND
title: Backends
status: normative
governs:
  - crates/agent/src/backend.rs
  - crates/bedrock/src/credentials.rs
  - crates/tui/src/app.rs
  - crates/config/src/bedrock.rs
  - crates/aichat/src/lib.rs
  - crates/aichat/src/models.rs
  - crates/config/src/provider.rs
  - crates/config/src/settings.rs
---

## Scope

Where a request for a reply goes. Three services can answer: the aichat endpoint Brave runs, Claude
on AWS Bedrock through somebody's own account, and an OpenAI-compatible gateway somebody configured.
This file governs which of them serves a given request, what a person is offered to choose from, and
what a configuration may decide.

The wire protocol of either service is ordinary code. So is signing, which
[network-egress.md](network-egress.md) covers as the one way out. What a reply is labelled once it
arrives is [labels.md](labels.md).

## Clauses

<a id="BACKEND-1"></a>
### BACKEND-1: a settings file may name a destination and never a permission

What a person's settings may say is which region, which credential profile, which host, which model
each tier names, which model to request when nobody has chosen one, and what to add to a request sent
to that host. Nothing in that file grants a capability, vouches for a path, or decides whether an
effect is allowed. It names no command to run.

The block does not become the process environment either. A value is consulted where a variable
would be, and reaches a subprocess only where that subprocess is the thing it configures.

**Why.** The file is read before anything runs and is the easiest thing on the machine to write to,
so a permission that could be granted from it would be a permission granted by whatever last edited
it. Installing the names globally would put every one of them in front of every command the agent
ever starts, which is a far larger claim than "this is how I reach the backend". A command named in
the file would be the largest claim of all, since running it is an effect nobody approved: that is
why a gateway block names a variable holding a credential rather than a way to produce one.

`verified-by: by-construction (values are consulted by name and never exported; the only value handed to a subprocess is the AWS profile, passed as an argument to the tool that owns it; no field is read as a path to execute, and a gateway's pass-through options reach a request body and nothing else)`

<a id="BACKEND-2"></a>
### BACKEND-2: configuring a second backend takes nothing away from the first

A settings block naming AWS tiers or a gateway does not change which model answers when nobody has
chosen one, and does not change how large a request may get before the conversation is shortened.

**Why.** Every build can reach Brave, and that is what somebody has before they configure anything.
Adding a way to reach more models should not quietly move the default onto one of them, nor set a
budget from a window that belongs to a model the session may never use.

`verified-by: bravebot_config::lib::a_bedrock_block_does_not_change_the_default_model`
`verified-by: bravebot_config::lib::a_bedrock_block_does_not_move_the_budget_off_the_default`
`verified-by: bravebot_config::lib::a_provider_block_changes_neither_the_default_model_nor_the_budget`

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
`verified-by: bravebot_agent::backend::a_configured_gateway_model_selects_the_gateway_backend`
`verified-by: bravebot_agent::backend::a_gateway_reports_the_model_it_was_asked_for`

<a id="BACKEND-4"></a>
### BACKEND-4: what a person may choose is every model any reachable service offers

Configuring a second backend puts its models on offer beside the first one's rather than in place of
them.

**Why.** A roster that replaced the other left somebody who named a single tier with a picker
offering exactly one model and no way back to the ones every build has. Reaching more models is not
a reason to stop reaching the existing ones.

`verified-by: bravebot_tui::app::configured_tiers_are_offered_alongside_the_brave_roster`
`verified-by: bravebot_tui::app::gateway_models_are_offered_alongside_the_other_rosters`
`verified-by: bravebot_agent::backend::a_gateway_does_not_take_the_other_rosters_away`

<a id="BACKEND-5"></a>
### BACKEND-5: only models that can actually be reached are offered

A tier appears when the configuration names a model for it, and not otherwise. A service's own
roster is offered only where this build holds the credentials to reach it. Nothing is invented for a
tier that was left unset, and a gateway nothing can authenticate is not asked for a roster nobody
could then use.

**Why.** A name on the list is a promise that picking it works. An ARN cannot be derived from a
model name, so an entry guessed for an unnamed tier is a choice that fails remotely, and a build
pointed only at AWS has no Brave credentials, so offering that roster would list models whose every
request fails unsigned.

`verified-by: bravebot_tui::app::a_tier_with_no_model_configured_is_not_offered`
`verified-by: bravebot_config::lib::without_brave_credentials_the_default_is_the_strongest_bedrock_tier`
`verified-by: bravebot_tui::app::only_the_gateway_models_the_file_named_are_offered`
`verified-by: bravebot_config::provider::a_provider_may_offer_no_models`
`verified-by: bravebot_config::provider::a_provider_without_a_base_url_is_not_offered`

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
`verified-by: bravebot_tui::app::a_gateway_row_says_which_service_answers_it`

<a id="BACKEND-7"></a>
### BACKEND-7: the conversation budget belongs to the model in force

How large a request may get before the conversation is shortened is taken from the model that will
answer it, at the moment that model is chosen.

**Why.** A budget above the real window does not shorten a conversation late, it stops shortening it
at all, silently: every round asks, no round qualifies, and the session runs to exhaustion looking
like one with nothing to summarise.

`verified-by: bravebot_tui::app::a_bedrock_entry_carries_the_window_the_budget_is_taken_from`
`verified-by: bravebot_tui::app::the_window_of_a_model_chosen_earlier_is_found_in_the_listing`
`verified-by: bravebot_tui::app::a_gateway_entry_carries_the_window_the_budget_is_taken_from`

<a id="BACKEND-8"></a>
### BACKEND-8: an unreachable listing costs only what it described

One service failing to say what it offers does not withdraw models known from configuration alone. A
choice is refused only when there is nothing left that could be chosen.

**Why.** Configured tiers need no network to know. Refusing the whole picker because one half was
unreachable would leave the only models this configuration can definitely reach unpickable, which is
the position somebody offline is most likely to be in.

`verified-by: bravebot_tui::app::an_unreachable_listing_still_offers_the_configured_tiers`
`verified-by: bravebot_tui::app::an_unreachable_listing_with_no_tiers_configured_is_still_a_failure`
`verified-by: bravebot_tui::app::an_unreachable_listing_still_offers_the_gateway_models`

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
`verified-by: bravebot_agent::backend::a_gateway_model_never_needs_an_aws_sign_in`

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

<a id="BACKEND-13"></a>
### BACKEND-13: a gateway block is read in the shape the tool that already reads it uses

What configures a gateway is a block whose field names, nesting and optionality are another tool's,
so that a block copied out of that tool's configuration works here unedited. Nothing is required that
it does not require, no field is added to the block however useful it would be, and a field this
system does not know is read past rather than refused.

**Why.** The whole value of the shape is that somebody already knows it and an editor already
validates it. A field added here would be one the other tool rejects, and a requirement added here
would refuse a block it accepts, so either one costs exactly the property the borrowing was for.
Reading past an unknown field is what makes a copy work, which is why it is the deliberate behaviour
and not a shortcoming.

Optionality is the part of the shape easiest to take and then quietly not honour. That tool resolves a
gateway's endpoint and its roster from a registry it fetches, so its blocks leave both out and the
commonest one names a credential and nothing else. Requiring either field here refuses a block it
accepts as surely as adding a field would.

`verified-by: bravebot_config::provider::a_provider_block_is_read`
`verified-by: bravebot_config::provider::a_model_entry_may_be_empty`
`verified-by: bravebot_config::provider::fields_this_crate_does_not_know_are_read_past`
`verified-by: bravebot_config::provider::a_limit_missing_either_half_states_no_window`
`verified-by: bravebot_config::settings::a_provider_block_is_read_beside_the_env_block`
`verified-by: bravebot_config::settings::a_provider_block_is_read_without_an_env_block`

<a id="BACKEND-14"></a>
### BACKEND-14: a window nobody stated is assumed low, never asked for and never guessed high

A configured model may state the size of its context window, and where it does not, the figure
assumed is one deliberately below what the model is likely to have. Nothing is asked over the network
to find out, and nobody is required to supply it.

**Why.** A budget above the real window does not shorten a conversation late, it removes shortening
altogether and silently. So the error has to lean low, which is the same reasoning already applied to
the single figure assumed for an opaque AWS profile. Requiring the number instead asks for one the
person writing the file does not have, since a window belongs to the model and the upstream serving
it, and a figure typed to satisfy a requirement looks authoritative in a way a default does not.
Asking the service costs a round trip before a picker can draw and breaks the case where nothing is
reachable.

Stating one still has to be possible, because one gateway serves models whose windows differ by more
than an order of magnitude, and pinning a request to a particular upstream can cap the window well
below what the model offers elsewhere.

`verified-by: bravebot_config::provider::a_model_without_a_stated_window_gets_the_assumed_one`
`verified-by: bravebot_config::provider::a_stated_window_is_read`
`verified-by: bravebot_tui::app::a_gateway_entry_carries_the_window_the_budget_is_taken_from`
`verified-by: bravebot_tui::app::a_gateway_model_with_no_stated_window_still_carries_one`

<a id="BACKEND-15"></a>
### BACKEND-15: what a gateway block adds to a request is carried, never interpreted

A configured model may carry a block of options that reaches the request body as it stands. Nothing
here parses it, knows what any of its fields mean, or validates them, and it cannot replace what the
turn itself put in the request.

**Why.** A gateway's routing controls are its own invention, so a schema enumerating them is one that
has to change when the gateway adds a field, and supporting the shape of gateways generally is what
makes this something other than support for one of them. It comes from the person's own configuration
surface and no model output reaches it, so it is trusted as far as a variable they exported would be.
That footing is what also bounds it: a destination may be named from a file, and a file overwriting
the model or the messages a turn built would be deciding what was asked rather than where it goes.

`verified-by: bravebot_aichat::lib::model_options_are_merged_into_the_request_body`
`verified-by: bravebot_aichat::lib::model_options_cannot_overwrite_what_the_turn_built`
`verified-by: bravebot_aichat::lib::a_model_with_no_options_adds_nothing_to_the_body`
`verified-by: bravebot_config::provider::model_options_are_carried_without_being_interpreted`

<a id="BACKEND-16"></a>
### BACKEND-16: a gateway credential is named rather than resolved ahead of time

Where a gateway's credential lives is named by its block: variables that may hold it, or a value
written in the file. It is read at the point a request needs it, and a request that cannot be
authenticated is refused with the remedy named rather than sent.

**Why.** Read once at startup, a credential goes stale in a session where somebody exported a new
one. Sent without one, the request fails at the far end for a reason nothing local could explain,
which is the same argument that stops a model name being guessed for an unconfigured tier. Naming a
variable is also the only way to keep a long-lived token out of a file people paste into issues,
which is why it is preferred where the block offers both and why a value in the file never displaces
one a variable holds.

`verified-by: bravebot_config::provider::a_named_variable_holds_the_token_before_the_file_does`
`verified-by: bravebot_config::provider::a_token_written_into_the_file_is_still_read`
`verified-by: bravebot_config::provider::a_provider_with_nothing_holding_a_token_has_none`
`verified-by: bravebot_agent::backend::a_gateway_with_nothing_holding_a_token_refuses_the_request`

<a id="BACKEND-17"></a>
### BACKEND-17: a gateway named by a name this system knows needs no endpoint written down

A block naming a gateway this system already knows an endpoint for reaches it without stating one. An
endpoint the block does state is where its requests go regardless. Where neither holds, the entry
configures no service.

The set of known names is compiled in. Nothing is fetched to resolve one, and no service is asked what
its own endpoint is.

**Why.** The tool this block's shape is borrowed from resolves an endpoint from a registry it fetches,
so a block copied out of it names one nowhere and requiring the field refuses a block that tool
accepts. That is exactly the property the borrowing exists for, and the shape is worth nothing if the
commonest block copied still has to be edited.

Compiled in rather than fetched because this value is where a bearer credential is sent. A service that
could decide it could have somebody's token by answering a request, which is the one thing a
destination may never be derived from, so the table is one somebody reviewed and shipped. Keeping it
short costs nothing: an absent name is served by writing the endpoint, so the price of not knowing a
gateway is a line of configuration rather than an unreachable service. A stated endpoint winning is
what keeps a known name usable against a proxy or a private deployment.

`verified-by: bravebot_config::provider::a_known_provider_name_supplies_its_own_endpoint`
`verified-by: bravebot_config::provider::a_stated_endpoint_beats_the_one_compiled_in`
`verified-by: bravebot_config::provider::a_provider_without_a_base_url_is_not_offered`

<a id="BACKEND-18"></a>
### BACKEND-18: a model name may say which gateway is meant, and only the rest of it is sent

A name selecting a gateway model may carry the gateway's own identifier ahead of the name that
gateway knows the model by, separated once. The identifier selects the service; only the remainder is
sent to it, and it is also what a reply's name is compared against. A name no configured gateway
claims reaches the service that does recognise it, as before.

**Why.** One model is reachable through more than one service, billed and credentialled differently,
and a bare name cannot say which was chosen. That mattered less while a gateway served only what a
settings file listed, because the file was the record of the choice. It decides correctness once a
roster is discovered rather than written down, since then nothing local can say which service a bare
name belonged to and a remembered choice would silently change service.

Sending only the remainder is what makes the qualified form usable at all: it is this system's own
filing, and the service has never heard of it. Comparing against the remainder too, because a reply
naming the model the gateway knows is the request working, and comparing against the qualified form
would report a substitution on every gateway turn. That is the same reasoning already applied to a
handle standing for whatever it resolves to.

Splitting once, rather than at every separator, because the remainder is the gateway's to spell and
most of those names contain one. A name whose leading segment matches no configured gateway is not a
qualified name at all, which is what keeps the other rosters' spellings out of this.

`verified-by: bravebot_config::lib::a_name_qualified_by_a_provider_id_names_the_gateway_and_the_model_separately`
`verified-by: bravebot_config::lib::a_bare_name_the_block_lists_still_finds_its_gateway`
`verified-by: bravebot_config::lib::a_name_no_gateway_was_configured_for_reaches_no_gateway`
`verified-by: bravebot_aichat::lib::only_the_name_the_gateway_knows_reaches_it`
`verified-by: bravebot_aichat::lib::a_qualified_name_still_finds_the_options_its_model_configured`
`verified-by: bravebot_tui::status::a_gateway_answering_under_its_own_name_is_not_a_substitution`

<a id="BACKEND-19"></a>
### BACKEND-19: a gateway that was told no models is asked what it serves

Where a gateway block names its models, those are what is offered and nothing is asked over the
network. Where it names none, the gateway itself is asked, and what it answers is offered. A listing
that cannot be fetched contributes nothing and takes nothing away from the rest of the roster.

Nothing is capped. Every model the gateway reports that can call tools is offered.

**Why.** A block naming no models is the ordinary case, not a mistake: the tool this shape is borrowed
from resolves a roster from a registry, so the commonest block copied in names a credential and
nothing else. Offering nothing for such a block means a gateway configured exactly as that tool
configures it appears in a diagnostic and is unusable, which was the state this replaced.

Asking only where nothing was named is what keeps the block worth writing. A stated roster costs no
round trip and works with no network, which is the position somebody offline is in, and it stays the
way to pin a short list out of a service that offers hundreds.

The listing is content and the pick is routing, the same footing the roster from Brave's endpoint
arrives on: names are drawn for a person, that person chooses, and their choice is the endorsement for
the field it lands in. What may not come from a service is where the request went, and that is
configuration here rather than anything fetched.

No cap, because a picker filters as somebody types and any limit is this system deciding they may not
choose a model their gateway serves. A window the gateway reports is taken, since it is the one fact
about a fetched model nobody can type, and a window the block stated outranks it as the figure
somebody pinned deliberately. Failing both, the same conservative default a stated roster gets.

`verified-by: bravebot_aichat::models::a_gateway_roster_is_offered_under_names_that_say_which_gateway_serves_them`
`verified-by: bravebot_aichat::models::a_window_a_gateway_reports_is_taken_from_the_listing`
`verified-by: bravebot_aichat::models::a_fetched_model_with_no_reported_window_gets_the_assumed_one`
`verified-by: bravebot_aichat::models::a_window_the_block_stated_outranks_the_one_reported`
`verified-by: bravebot_aichat::models::a_gateway_model_that_cannot_call_tools_is_not_offered`
`verified-by: bravebot_aichat::models::a_gateway_that_reports_no_capabilities_still_offers_its_models`
`verified-by: bravebot_aichat::models::a_fetched_entry_with_no_usable_name_is_dropped`
`verified-by: bravebot_aichat::models::fetched_gateway_models_are_not_marked_premium`

## Known costs

- **A credential is resolved by running the AWS CLI.** Reaching Bedrock needs short-lived keys that
  expire during a session, and the tool that holds them is the one the person already signs in
  with. That is a process this code did not write, reading a configuration this code does not
  govern.

- **A gateway credential may be a plaintext string in the settings file.** The shape this block
  borrows has a field for one, and taking the shape means taking the field. Naming a variable is
  recommended and preferred where both are present, but nothing prevents the other, and the only
  real fix is a credential store this does not have.

- **A gateway's pass-through options are unvalidated.** A misspelled routing field is a request the
  gateway rejects, or worse one it silently routes somewhere unintended. The alternative is a schema
  that goes stale as the gateway changes, and that trade is what keeps this from being support for
  one particular gateway.

- **Fields another tool defines are read past in silence.** Somebody who knows the shape will expect
  its cost, modality and package fields to do something here, and they do nothing. That surprise is
  the price of a block that can be copied in either direction.

- **The assumed AWS window is a guess.** No endpoint there reports a context window, and an
  inference-profile ARN does not say which model it resolves to, so one figure stands in for every
  tier: the one an unresolvable profile actually gets. It is deliberately low, because being wrong
  upward removes shortening rather than delaying it.
