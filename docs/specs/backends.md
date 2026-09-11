---
id: BACKEND
title: Backends
status: normative
governs:
  - crates/agent/src/backend.rs
  - crates/bedrock/src/credentials.rs
  - crates/tui/src/app.rs
  - crates/config/src/bedrock.rs
  - crates/config/src/lib.rs
  - crates/aichat/src/lib.rs
  - crates/aichat/src/models.rs
  - crates/config/src/provider.rs
  - crates/config/src/settings.rs
---

## Scope

Where a request for a reply goes. Three services can answer: the aichat endpoint Brave runs, AWS
Bedrock through somebody's own account, and an OpenAI-compatible gateway somebody configured.
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
to that host. Nothing in a settings file grants a capability or vouches for a path. None of them names
a command to run.

The block does not become the process environment either. A value is consulted where a variable
would be, and reaches a subprocess only where that subprocess is the thing it configures.

**Why.** These files are read before anything runs and are the easiest thing on the machine to write
to, so a capability that could be granted from one would be a capability granted by whatever last
edited it. That holds hardest for the layers a checkout carries, which arrive with the checkout.
Installing the names globally would put every one of them in front of every command the agent ever
starts, which is a far larger claim than "this is how I reach the backend". A command named in a file
would be the largest claim of all, since running it is an effect nobody approved: that is why a
gateway block names a variable holding a credential rather than a way to produce one.

**Note.** A `permissions` block is not an exception. Its rules only ever narrow what would otherwise
be allowed, and nothing in one grants an effect that was refused without it.

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

Where the same model is reachable through more than one service, every model offered carries which
service answers it, in terms that cannot collide with a name a service chose for itself. Brave's own
roster carries nothing, being the one every build has. What a person reads is drawn from that,
under [VIEW-15](terminal-transcript.md#VIEW-15), and the note left when a model is chosen says it
too, since the row that carried it is gone by the time that note is read.

**Why.** The two are billed differently and authenticate differently, so which one answers is the
whole of what is being chosen between. Naming the service is not enough: Brave serves part of its own
roster through Bedrock and says so in the names it sends, so that word appeared on both halves of the
list and distinguished nothing. Carried beside the name rather than composed into it, because a
roster of several services is read as sections, and a service composed into every name is that name
repeated down a hundred rows.

`verified-by: bravebot_tui::app::a_configured_tier_is_not_confusable_with_a_brave_model_served_through_bedrock`
`verified-by: bravebot_tui::app::a_tier_with_no_profile_configured_still_names_the_account`
`verified-by: bravebot_tui::app::a_gateway_row_says_which_service_answers_it`
`verified-by: bravebot_aichat::models::a_fetched_row_carries_the_gateway_that_serves_it`

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

A kept answer is dropped the moment a request proves it wrong, and the next check runs the tool
again.

**Why.** The check happens before every turn, and the tool that answers it takes most of a second, so
paid each time it is a pause between pressing Enter and seeing the line appear. The expiry is the
credential's own word about how long the answer stays true, which is why it and not a fixed interval
is what bounds this. Stopping short of it matters because the answer is used to decide whether to
sign in before work that then has to be signed: taken at the last second, the request that follows
carries a credential that has already expired.

An expiry is what the credential says, not a promise. A session can be revoked, ended from another
machine, or lose what it granted, and then a kept answer is wrong before the time it named. Held on
to, it is worse than never having cached at all: every check before every turn repeats the stale yes,
so the sign-in that a person can see never runs, and each turn instead fails to one that reports to
nobody. Dropping it on the first request that disproves it is what keeps the caching an optimisation
rather than a way to get stuck.

`verified-by: bravebot_bedrock::credentials::a_session_already_shown_to_be_good_is_not_asked_about_again`
`verified-by: bravebot_bedrock::credentials::a_session_that_has_run_out_is_asked_about_again`
`verified-by: bravebot_bedrock::credentials::a_session_about_to_run_out_is_treated_as_already_gone`
`verified-by: bravebot_bedrock::credentials::one_profile_being_good_says_nothing_about_another`
`verified-by: bravebot_bedrock::credentials::the_default_profile_is_remembered_like_any_other`
`verified-by: bravebot_bedrock::credentials::a_session_with_no_stated_expiry_is_not_kept`
`verified-by: bravebot_bedrock::credentials::an_expiry_is_converted_to_the_instant_it_names`
`verified-by: bravebot_bedrock::credentials::the_expiry_the_cli_reports_is_read_from_the_process_format`
`verified-by: bravebot_bedrock::credentials::an_expiry_that_is_not_the_expected_shape_is_not_guessed_at`
`verified-by: bravebot_bedrock::credentials::a_session_shown_to_be_bad_is_no_longer_remembered_as_good`
`verified-by: bravebot_bedrock::credentials::forgetting_one_profile_leaves_the_others_alone`

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
is that the service owns those names, and a renamed one is reset by the endpoint to
`automatic-brave-bot`, which is where somebody with no `model` key already starts.

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

A configured gateway also works in a development build without Brave service credentials. Those
credentials remain required when neither a gateway nor Bedrock is configured. A gateway's token is
resolved when it is used; its absence is a gateway authentication error, not a request for Brave keys.
The selected model names the gateway, for example `openrouter/z-ai/glm-4.6` in the top-level `model`
key of `~/.bravebot/settings.json`. Adding a gateway does not replace a configured Brave or Bedrock
backend or override the chosen model.

`verified-by: bravebot_config::lib::a_gateway_configures_without_brave_credentials_or_a_model_roster`
`verified-by: bravebot_config::lib::a_gateway_can_read_its_own_environment_token_without_brave_credentials`
`verified-by: bravebot_config::lib::an_absent_or_invalid_gateway_does_not_relax_brave_validation`
`verified-by: bravebot_config::lib::a_gateway_keeps_bedrock_available_without_brave_credentials`

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

**An entry naming AWS is the exception**, and not a token kept somewhere else: Bedrock takes a
signature over the request, so there is no credential for a block to name and none is read from one.
Which set of AWS credentials to sign with comes from the profile the block names, resolved when a
request needs it, which is the same moment and the same reason a token is read.

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

What the credential in use may reach is asked for ahead of what the service offers generally, and the
wider roster answers only where the narrower question does not. Nothing is capped: every model
reported that can call tools is offered, ordered with the model a session would use first and the rest
by name.

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

Asking what the credential may reach is asking the question a person actually has. A model their key
cannot serve is a row that fails the moment it is picked, and the two answers differ by a factor of
three, so the wide roster is mostly rows that would not work. It is a fallback rather than the only
request because that narrower route is a gateway's own extension: one that does not answer it has to
end up with a roster anyway.

No cap, because a picker filters as somebody types and any limit is this system deciding they may not
choose a model their gateway serves. Ordering does that work instead, and it is needed precisely
because nothing is dropped: a roster arriving newest-first opens on models nobody has heard of and
buries the one in use. A configured roster is left alone, the file being the order somebody chose.

A window the gateway reports is taken, since it is the one fact about a fetched model nobody can type,
and a window the block stated outranks it as the figure somebody pinned deliberately. Failing both,
the same conservative default a stated roster gets.

`verified-by: bravebot_aichat::models::a_gateway_roster_is_offered_under_names_that_say_which_gateway_serves_them`
`verified-by: bravebot_aichat::models::a_window_a_gateway_reports_is_taken_from_the_listing`
`verified-by: bravebot_aichat::models::a_fetched_model_with_no_reported_window_gets_the_assumed_one`
`verified-by: bravebot_aichat::models::a_window_the_block_stated_outranks_the_one_reported`
`verified-by: bravebot_aichat::models::a_gateway_model_that_cannot_call_tools_is_not_offered`
`verified-by: bravebot_aichat::models::a_gateway_that_reports_no_capabilities_still_offers_its_models`
`verified-by: bravebot_aichat::models::a_fetched_entry_with_no_usable_name_is_dropped`
`verified-by: bravebot_aichat::models::fetched_gateway_models_are_not_marked_premium`
`verified-by: bravebot_tui::app::a_fetched_roster_leads_with_the_model_in_force`
`verified-by: bravebot_tui::app::a_fetched_roster_nobody_has_chosen_from_is_still_sorted`

<a id="BACKEND-20"></a>
### BACKEND-20: how hard to think is carried, never inferred

A request says how hard the model should think only where a turn was given a level. Nothing derives
one from the prompt, from how long the conversation is, from which tools are offered, or from what a
previous turn cost. Where no level was given the field is absent and the service applies its own
default, so a build nobody has asked sends the body it always sent.

**Why.** How hard to think is a bill, and inferring one is this program spending somebody's money on
a guess about work it has not done yet. The absent case is what keeps the field honest: an endpoint
that has never seen it is not sent it, so adding it cannot break a service that would reject it.

A level is not content. It comes from a person picking off a list they read, on the footing the
model name beside it arrives on, and a word that names no level is no choice at all rather than a
choice of something.

`verified-by: bravebot_aichat::protocol::a_request_nobody_asked_a_level_of_mentions_no_effort`
`verified-by: bravebot_aichat::protocol::a_word_naming_no_level_is_not_a_choice`
`verified-by: bravebot_aichat::protocol::a_level_is_named_whatever_case_it_was_written_in`
`verified-by: bravebot_bedrock::protocol::a_request_nobody_asked_a_level_of_carries_no_output_config`
`verified-by: bravebot_agent::turn::without_a_chosen_effort_no_level_is_requested`


<a id="BACKEND-21"></a>
### BACKEND-21: each service is sent the level in its own protocol's field

One word ranks the levels, and where that word goes in the body is the wire protocol's business. No
service is sent the other's shape.

| Service | Where the level is sent |
|---|---|
| The aichat endpoint Brave runs | `reasoning_effort`, beside the model |
| An OpenAI-compatible gateway | `reasoning_effort`, beside the model |
| AWS Bedrock | `effort`, inside `output_config`, among the fields handed to the model unread |

This says where a level is sent, not what becomes of it. What a service does with the field is that
service's own behaviour, observable only by measuring it, and one of the three is known to discard it
altogether. That is recorded under Known costs rather than here, because a clause pinned to a remote
service's current behaviour is a clause that goes stale without anything in this repository changing.

**Why.** The two protocols state the same idea differently, and the conversion between them already
happens in one place for every other field. Sending one service the other's shape is a field it
certainly does not read.

`verified-by: bravebot_aichat::protocol::a_level_is_sent_in_the_name_this_protocol_gives_the_field`
`verified-by: bravebot_bedrock::protocol::a_level_is_sent_inside_the_object_this_api_states`
`verified-by: bravebot_agent::turn::a_chosen_effort_is_the_one_requested`


<a id="BACKEND-22"></a>
### BACKEND-22: a level is sent only where the roster says it is read

Where the listing describing the model in force states which parameters it takes and an effort level
is not among them, no level is sent and the person is told the model reads none. The choice itself is
kept: it applies again the moment a model that reads one is chosen, so what a request carries does
not depend on the order two commands were typed in.

A model the listing does not describe is not a model stated to read nothing. A name that came from a
settings file, a roster that reports no parameters at all, and a listing that could not be fetched
all leave the level to go out and be judged at the far end.

**Why.** A service that reads the field and one that discards it are indistinguishable from the
outside: both answer, and the reply of a model that ignored the level looks exactly like the reply of
one that honoured it. So a level sent where it is not read is a charge somebody chose and did not
get, reported to them as in force. Where a roster answers the question there is no reason to guess,
and where it does not, withholding what somebody asked for on the strength of a listing that never
mentioned the subject would be deciding against them from silence.

`verified-by: bravebot_aichat::models::a_gateway_model_that_does_not_take_the_effort_parameter_says_so`
`verified-by: bravebot_aichat::models::a_gateway_that_states_no_parameters_is_not_taken_to_read_no_level`
`verified-by: bravebot_tui::app::a_level_is_withheld_from_a_model_that_reads_none`
`verified-by: bravebot_tui::app::a_level_a_model_cannot_use_is_kept_rather_than_forgotten`
`verified-by: bravebot_tui::app::asking_for_a_level_a_model_cannot_use_says_so`
`verified-by: bravebot_tui::app::a_model_the_listing_did_not_describe_still_takes_a_level`
`verified-by: bravebot_tui::status::a_level_the_model_does_not_read_is_reported_as_unread`


<a id="BACKEND-23"></a>
### BACKEND-23: a profile that is not configured is said so, never signed in to

Before a sign-in is attempted for a named profile, the tool is asked which profiles it has. Where it
answers and the name is not among them, no sign-in runs and the person is told the name is not
configured, along with the ones that are. Where no profile was named, or the tool could not be asked,
the sign-in goes ahead.

The question is asked of the tool as a list, never inferred from the wording of the failure that
prompted it, and only once something has already failed, so establishing that a session is good still
costs one call and no more.

**Why.** No sign-in fixes a profile that does not exist: it fails for the same reason the export did,
which spends a browser on a certainty and then replaces the real diagnosis with "the sign-in did not
complete". The remedy is also different, and advice to run `aws sso login` is advice that cannot
work, so a person following it learns nothing. Naming the profiles that do exist is what turns the
report into something actionable, since the usual cause is a name that was right on another machine.

Reading the failure's wording would answer the same question, and is how this must not be done: the
wording belongs to a tool that may change it, and being wrong costs either a browser opened for
nothing or somebody told their configuration is broken when their session merely expired.

`verified-by: bravebot_bedrock::credentials::a_profile_the_cli_does_not_have_is_not_signed_in_to`
`verified-by: bravebot_bedrock::credentials::a_profile_the_cli_does_have_still_gets_a_sign_in`
`verified-by: bravebot_bedrock::credentials::a_listing_that_could_not_be_read_does_not_withhold_a_sign_in`
`verified-by: bravebot_bedrock::credentials::naming_no_profile_is_not_naming_a_missing_one`
`verified-by: bravebot_bedrock::credentials::a_machine_with_no_profiles_at_all_says_so`


<a id="BACKEND-24"></a>
### BACKEND-24: three settings layers resolve a name at a time, closest first

Settings are read from three files: `settings.json` in the user's own directory, then
`settings.json` in a `.bravebot` directory beside the work, then `settings.local.json` beside that
one. A later file overrides an earlier one per name rather than wholesale, so a file setting one thing
leaves everything else in force.

| What | How layers combine |
|---|---|
| `env`, `provider` | per name, one level down; the value under a name is replaced whole |
| `run.scrubEnv`, every list under `permissions` | every layer's entries are kept |
| `model`, anything else | the closest layer that set it wins |

The project layers are read from the directory the process started in and no ancestor of it. Each
layer fails independently: one that is missing, oversized, or unparseable leaves the others in force.

**Why.** An account is not the only scope a value belongs to. A credential profile is a property of
the person, the gateway a particular checkout talks to is a property of that checkout, and something
one machine needs is neither, so a single file makes one of those three overwrite the others.
Overriding per name is what makes putting one value in a checkout worth doing, since the alternative
is restating an entire configuration to change a host. Going deeper than a name would make one
request's destination the product of two files with no single place to read that says where it goes,
which is why a gateway entry is replaced whole and a project file naming one must name its host too.

The lists are the exception because an entry in one only ever narrows what is possible: a name under
`scrubEnv` takes a variable away from a subprocess, and a rule under `permissions` refuses something
that was otherwise allowed. Overriding either would let a layer hand back what a weaker one withheld,
and a permission removed by a file somebody did not open is the one outcome worth ruling out. A model
is one choice rather than a list, so it resolves like any other single value.

Searching upward for the project layer is what this declines to do, because then what configures a
session would depend on which directory somebody happened to change into, and the file found could sit
above the thing being worked on. Refusing the whole stack over one bad layer is the other thing it
declines: a mistake in a checkout must not decide that somebody's own profile no longer applies.

The order and the merge rules are Claude Code's, down to the name `settings.local.json`, so that
knowing where to put a value for one tool is knowing it for the other.

`verified-by: bravebot_config::settings::a_project_layer_overrides_a_name_the_global_one_set`
`verified-by: bravebot_config::settings::a_name_only_the_global_layer_set_survives_a_project_layer`
`verified-by: bravebot_config::settings::the_local_layer_beats_the_one_a_checkout_carries`
`verified-by: bravebot_config::settings::every_layer_adds_to_the_names_kept_from_a_program`
`verified-by: bravebot_config::settings::every_layer_adds_to_the_permission_rules`
`verified-by: bravebot_config::settings::every_layer_adds_to_the_directories_a_file_makes_reachable`
`verified-by: bravebot_config::settings::the_closest_layer_that_named_a_model_wins`
`verified-by: bravebot_config::settings::a_layer_naming_no_model_leaves_the_one_below_it`
`verified-by: bravebot_config::settings::a_project_layer_replaces_one_gateway_and_leaves_the_others`
`verified-by: bravebot_config::settings::a_project_gateway_naming_no_host_replaces_one_that_did`
`verified-by: bravebot_config::settings::an_unparseable_project_layer_leaves_the_global_one_in_force`
`verified-by: bravebot_config::settings::an_oversized_project_layer_leaves_the_global_one_in_force`
`verified-by: bravebot_config::settings::a_directory_with_no_project_layer_reads_the_global_one_alone`
`verified-by: bravebot_config::settings::the_layers_that_were_read_are_reported_weakest_first`
`verified-by: bravebot_config::settings::a_name_more_than_one_layer_set_reports_the_file_that_won`
`verified-by: bravebot_config::settings::an_override_reports_the_name_and_the_file_and_never_the_value`
`verified-by: bravebot_config::lib::the_environment_outranks_the_settings_file`


<a id="BACKEND-25"></a>
### BACKEND-25: a request to Brave's endpoint says which product is asking

Every request to the aichat endpoint Brave runs carries `Brave-Product: brave-bot`, both the one
asking for a reply and the one asking what models exist. The endpoint answers a listing curated for
this product, and resolves that product's own automatic entry. Nothing decides whether to send the
header: it is a literal on the one path that builds a request to that endpoint.

A gateway is sent no such header, and neither is a gateway's roster request. What a third-party
service is sent is the shape it documents, and a header naming a product it has never heard of is
this system telling it something it cannot act on.

**Why.** The endpoint serves Leo as well, whose roster is chosen for a chat assistant. A good
fraction of it cannot call tools at all, and choosing one of those produces an agent that can read
and write nothing, which is the promise BACKEND-5 makes about a name on the list. Asking as a product
is what makes that promise keepable without this code carrying a hand-written list of which models
are suitable, since a list compiled in here goes stale the moment the roster changes and the service
is the thing that knows.

`verified-by: bravebot_aichat::lib::a_request_without_a_gateway_is_still_signed`
`verified-by: bravebot_aichat::lib::a_gateway_request_goes_to_the_configured_host_with_a_bearer_token`
`verified-by: bravebot_aichat::client::the_model_listing_is_fetched_from_the_models_path`


<a id="BACKEND-26"></a>
### BACKEND-26: the automatic entry names this product, and Leo's name resolves to it

The name requested when nobody has chosen a model is `automatic-brave-bot`. Where a settings file,
an exported variable or a choice recorded earlier says `automatic`, that name resolves to
`automatic-brave-bot` before it reaches a request. Every other name is used as written, under
BACKEND-12.

**Why.** `automatic` is Leo's triage entry and routes by a policy chosen for a chat assistant, so the
two words name different behaviour rather than the same behaviour twice. A recorded choice outlives
the version that made it and a settings file is copied between machines, so the older name is in
circulation and reaching the endpoint. Left alone it is not refused: it selects the other product's
routing, which is a working request that quietly is not what this agent asked for.

**Note.** The resolution is one-way, so `automatic` cannot be requested. Reaching Leo's routing
deliberately is not something the picker offers, that entry not being on this product's roster.

`verified-by: bravebot_config::lib::the_legacy_automatic_name_becomes_the_brave_bot_default`
`verified-by: bravebot_tui::store::the_legacy_automatic_name_is_rewritten_on_read`
`verified-by: bravebot_aichat::models::automatic_is_offered_once_even_if_the_server_lists_it_too`
`verified-by: bravebot_agent::turn::without_a_choice_the_configured_default_is_requested`

<a id="BACKEND-27"></a>
### BACKEND-27: a Bedrock request marks the prefix it will send again

Two cache breakpoints go on every request to Bedrock: one at the end of the system prompt, which
covers the tool schemas in front of it, and one on the last block of the conversation, which moves
to the end as the conversation grows. Neither changes what is sent, only what the service has to
read again.

**Why.** A turn re-sends its whole history every round. One session reached 104,633 tokens over
twenty-seven rounds and paid for every token of every round at full price, in latency as much as in
money, and the great majority of those tokens were bytes the service had already read a minute
earlier. The prompt and the schemas are identical on every round of every session; the messages in
front of the last one are identical to a round ago.

**Rolling rather than fixed.** Each request writes the round before it into the cache and reads
back everything older, which is what makes the second breakpoint worth a cache write. A conversation
ending in an image or a tool call is left with the breakpoint on the system prompt alone, and costs
a cache write and nothing else.

**The reported prompt is what was sent, not what was read.** This API states `inputTokens` net of
the cache and reports the cached tokens beside it, so the three are added back together on the way
into a `Usage`. Without that a cached round reads as a conversation that shrank while it grew.

**A model that refuses them is not sent them again.** Prompt caching is not something every model
this backend can reach offers, and one that does not refuses the whole request rather than reading
past the breakpoints. A request refused on its contents is therefore sent once more without them,
and where that answers, no later request in the session carries them. Only a refusal does this:
every other failure leaves the breakpoints in place.

**Why ask rather than know.** An inference-profile ARN does not say which model is behind it, which
is the same fact that makes one figure stand in for every tier's context window. Asking costs one
extra round trip the first time such a model is used; not asking costs every request to it, since
the breakpoints are a part of the request nobody asked for and the service refuses the lot.

**Only this backend.** The aichat endpoint and an OpenAI-compatible gateway take a different wire
format, which has no field for this and asks for nothing.

`verified-by: bravebot_bedrock::protocol::the_system_prompt_carries_a_breakpoint`
`verified-by: bravebot_bedrock::protocol::the_last_block_of_the_conversation_carries_a_breakpoint`
`verified-by: bravebot_bedrock::protocol::a_conversation_ending_in_a_tool_result_is_marked_too`
`verified-by: bravebot_bedrock::protocol::a_reply_without_a_breakpoint_still_parses`
`verified-by: bravebot_bedrock::protocol::cached_tokens_are_counted_as_the_prompt_they_were`
`verified-by: bravebot_bedrock::protocol::a_request_without_breakpoints_keeps_everything_that_was_asked_for`
`verified-by: bravebot_bedrock::lib::a_request_refused_on_its_contents_is_asked_again_without_the_breakpoints`
`verified-by: bravebot_bedrock::lib::only_a_refusal_on_the_contents_drops_the_breakpoints`

<a id="BACKEND-28"></a>
### BACKEND-28: a Bedrock tier may name any model that account can reach

A request to AWS Bedrock is built the same way whatever model it names, in the body that service
states for every provider it hosts rather than in any one provider's own. A tier may therefore name
a model from any of them, and an inference profile standing for one, without anything here
recognising which provider is behind it.

**Why.** Bedrock fronts several providers, and an inference-profile ARN does not say which one
serves it. A body shaped for a single provider therefore makes which models are reachable a
property of this code rather than of the account: the name is accepted, signed, sent, and refused at
the far end on the body, which is the failure BACKEND-3 argues against for a name no service
recognises. It is worse here, because the account can reach the model and nothing a person writes
in a settings file closes the gap. Where a role is scoped to inference profiles, which is how
per-user cost allocation is granted, the routes a bearer token can use refuse those profiles
outright, so this is the only one that answers at all.

**Note.** The tier words stay `opus`, `sonnet` and `haiku`. They name a slot in a settings file
rather than a model family, and a tier is whichever model the account named for it.

`verified-by: bravebot_bedrock::protocol::the_request_names_no_provider_of_its_own`
`verified-by: bravebot_config::bedrock::streaming_and_buffered_requests_have_different_routes`
`verified-by: bravebot_config::bedrock::a_model_arn_is_encoded_into_the_path`

<a id="BACKEND-29"></a>
### BACKEND-29: a settings block may name an AWS account, and every model it lists is reachable

A `provider` entry may name AWS Bedrock instead of an OpenAI-compatible gateway. It states a region,
optionally a credential profile, and as many models as the file lists, each reached by the name it
is keyed under. Such an entry is served by the Bedrock backend and signed with the AWS credential
chain, never sent to a gateway, and it adds to what the tier variables already name rather than
replacing it.

**Why.** The tier variables are three, named for one provider's model families, and a fourth model
can only be had by giving up one of the three. That is a limit of the shape those variables have,
not of the account, which reaches as many models as it is entitled to. The block this borrows is
already how the other tool configures the same account, keyed by model rather than by tier, so
taking it costs nothing and removes the limit.

Signed rather than authenticated with a token because that is what the service takes, and it is
also what makes the entry worth having: the routes a bearer token can use refuse the inference
profile ARNs a per-user role is commonly scoped to, so a gateway entry pointed at the same account
reaches nothing.

**The region is required and the endpoint is not.** The host carries the region, so one value
produces the other and there is nothing left to state. An entry naming no region configures no
service, for the reason a Bedrock block without one does not: a guessed region is a request that
fails somewhere far from the mistake.

**What a row says.** A model named here has no tier, so a picker row carries what the block called
it, and its id where the block called it nothing. An inference-profile ARN is not a name anybody
reads, which is why this is worth stating rather than left to fall out of the id.

`verified-by: bravebot_config::provider::an_aws_block_configures_bedrock_rather_than_a_gateway`
`verified-by: bravebot_config::provider::an_aws_model_the_block_did_not_name_is_shown_by_its_id`
`verified-by: bravebot_config::provider::an_aws_block_without_a_region_configures_nothing`
`verified-by: bravebot_config::lib::a_model_an_aws_block_named_reaches_bedrock_rather_than_a_gateway`
`verified-by: bravebot_config::lib::a_name_qualified_by_the_aws_id_is_not_a_gateway_either`
`verified-by: bravebot_agent::backend::a_model_an_aws_block_named_selects_the_bedrock_backend`
`verified-by: bravebot_tui::app::a_bedrock_model_a_block_named_is_shown_under_that_name`

## Known costs

- **The effort level is the one field in a Bedrock request that a single provider defines.** The
  body Bedrock states for every provider it hosts has no field for how hard to think, so the level
  travels in the field that service hands to the model without reading, spelled the way the
  Anthropic API spells it. A tier naming a model from another provider is reachable and answers,
  and a level chosen against one is refused by that model on the field name. Nothing here can tell
  the two apart, an inference-profile ARN not saying which provider serves it, and the alternative
  is withholding a level from every Bedrock model including the ones that read it.

- **Which models a product is served is the service's decision, and this holds no copy of it.** The
  roster is whatever the endpoint returns for `brave-bot`, so a model becoming unsuitable for agentic
  work is a change nothing here would notice, and one wrongly dropped from the curated set is a model
  a person cannot pick however well it would have worked. The alternative is a list of names compiled
  in, which BACKEND-12 already declines for the tier words and for the same reason: the service owns
  the names, and a copy here is a copy that goes stale.

- **A layer a checkout carries is trusted as far as the person's own file is.** A `.bravebot`
  directory arrives with whatever produced the checkout, so a `settings.json` in one can name the host
  every request goes to and the credential that signs it, and somebody who has not read it would not
  know. Nothing here distinguishes the layers, because the resolution being copied does not, and what
  limits the damage is the same rule that limits it anywhere: a file names a destination and grants no
  capability, so the worst it does is send a request somewhere useless or somewhere watching. Refusing
  the fields that name a destination in the project layers is the fix if that trade stops being worth
  it, and it would cost the main reason to put a value in a checkout at all.

- **The aichat endpoint Brave runs discards the effort level.** Measured against that endpoint: a
  nonsense value in `reasoning_effort` is answered `200` with usage identical to a request that omits
  the field, so it is not validated, and on `near-glm-5`, which reports a non-zero reasoning-token
  count for an ordinary prompt, that count does not move with the level. The premium rows of the
  roster were not measured, a free-tier credential being substituted to a weaker model before the
  request lands, so nothing here is established about them. A level chosen against a Brave-served
  model is therefore carried, sent, and dropped, while the interface goes on reporting it as in
  force. Bedrock is unaffected, the level reaching the model in the field that model defines.

- **A level a service does advertise may still not mean what this sends.** `xhigh` and `max` are
  levels the Anthropic API defines, and a gateway row advertising `reasoning_effort` says it reads
  the parameter without saying which words it accepts. A model may reject or silently round a level
  it does not know, and no listing distinguishes that from honouring it.

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
