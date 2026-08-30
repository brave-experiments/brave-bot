---
id: COMPACT
title: Compacting a conversation
status: normative
governs:
  - crates/agent/src/compact.rs
  - crates/agent/src/conversation.rs
guards:
  - symbol: Policy::adopt_summary
---

## Scope

Every round re-sends the whole conversation, so a long session grows its own request until the
server refuses it. Compaction replaces the older part of the exchange, **in the request only**,
with a summary of it. `/compact` asks for the same thing on demand.

## Why the summariser is not a processor

Compaction is never routed through a processor.

What licenses the call instead is that there is nothing new to read. Every message in a
conversation has already been past the gate that decides what the planner may see: either it was
judged trusted and shown, or what went in was a reference and the bytes stayed in quarantine. So
the summariser's context **is** the planner's context, and its answer is labelled from that
context exactly as the planner's own words are. Nothing is upgraded.

## Clauses

<a id="COMPACT-1"></a>
### COMPACT-1: a summary is adopted only while the context is trusted

Once the context has gone untrusted the summary is refused and the conversation is left exactly as
it was. It is not quarantined instead, and it is not relabelled to get past the gate.

**Why.** A reference to the planner's own history is not a history. A conversation that cannot be
shortened stays long, which is the one outcome here that is never wrong.

**This cannot happen today.** A context only becomes untrusted by resuming one that already was,
and nothing makes one untrusted in the first place: untrusted content is quarantined rather than
shown, and the only place the context absorbs anything is where content was trusted enough to show.
The gate is here anyway, because what makes that closure safe to rely on is that something refuses
if it ever stops holding. If a change ever lets untrusted bytes into the planner's context, this is
what catches it.

`verified-by: bravebot_core::policy::a_summary_of_a_trusted_conversation_is_adopted`
`verified-by: bravebot_core::policy::a_summary_of_an_untrusted_conversation_is_refused_rather_than_adopted`
`verified-by: bravebot_agent::turn::a_summary_of_an_untrusted_conversation_leaves_the_conversation_whole`

<a id="COMPACT-2"></a>
### COMPACT-2: the summariser is offered no tools

It is asked for text and given nothing to act with.

`verified-by: bravebot_agent::turn::the_summariser_is_offered_no_tools`
`verified-by: bravebot_agent::turn::compacting_on_request_grants_itself_nothing_but_reaching_the_model`

<a id="COMPACT-3"></a>
### COMPACT-3: three things compaction never touches

The **quarantine**, which holds the only copy of what a surviving reference names; the **reference
counter**, since slots are written once and a name handed out twice would collide; and the
**integrity**, since nothing here has un-read what the conversation read.

None of these may be relaxed to save room.

`verified-by: bravebot_agent::conversation::a_reference_minted_before_compaction_still_names_its_content_after_it`
`verified-by: bravebot_agent::conversation::compaction_does_not_rewind_the_reference_counter`
`verified-by: bravebot_agent::conversation::compaction_does_not_restore_integrity_the_conversation_had_lost`

<a id="COMPACT-4"></a>
### COMPACT-4: the cut never lands inside a round

A call is never separated from its results, and a round in progress is not a place to cut. Whole
exchanges are given up first, since the boundary between two of them is the one a person would
draw. A turn that has gone long by itself has no earlier exchange to give, because it adds one
prompt however many rounds follow, so it gives up earlier **rounds** instead.

**Why.** A cut between a call and its results leaves the head saying the call never ran and the
tail holding an answer to a call that is not there.

`verified-by: bravebot_agent::conversation::compaction_never_separates_a_call_from_its_results`
`verified-by: bravebot_agent::conversation::a_round_in_progress_is_never_a_place_to_cut`
`verified-by: bravebot_agent::conversation::compaction_keeps_the_most_recent_exchanges_word_for_word`
`verified-by: bravebot_agent::turn::a_long_turn_summarises_its_earlier_rounds_partway_through`

<a id="COMPACT-5"></a>
### COMPACT-5: a cut must give up at least as much as it keeps

Summarising costs a model call, and a request has a floor it cannot go below: the system prompt and
the tool schemas. A budget under that floor is unreachable however much history is given up, so a
cut that would not free more than it retains is not made at all.

**Why.** Without it, a turn in that position summarises itself once per round for the rest of its
life and shortens nothing. Measured at 35 summaries in a turn that should have made none. Never
relax this to compact sooner.

`verified-by: bravebot_agent::conversation::a_cut_that_would_give_up_less_than_it_keeps_is_not_worth_a_request`
`verified-by: bravebot_agent::conversation::a_long_turn_does_not_summarise_itself_once_per_round`
`verified-by: bravebot_agent::conversation::a_head_that_is_only_an_earlier_summary_is_not_compacted_again`
`verified-by: bravebot_agent::conversation::compacting_forgets_a_measurement_of_the_conversation_it_replaced`
`verified-by: bravebot_agent::conversation::a_conversation_with_nothing_but_recent_exchanges_is_not_compacted`
`verified-by: bravebot_agent::turn::a_conversation_nobody_has_measured_is_not_compacted`

<a id="COMPACT-6"></a>
### COMPACT-6: the request is shortened, never the record

The replaced messages go to an archive the transcript still reads and the session record still
stores. The user owns their transcript; compaction is about what gets sent.

`verified-by: bravebot_agent::conversation::what_compaction_took_out_of_the_request_is_still_recounted_to_the_person`
`verified-by: bravebot_agent::conversation::a_compacted_conversation_survives_being_written_down`
`verified-by: bravebot_agent::conversation::the_summary_is_not_shown_as_something_the_user_said`

<a id="COMPACT-7"></a>
### COMPACT-7: the budget is configurable, and a nonsensical one falls back

A budget that makes no sense falls back to the default rather than disabling compaction, so a
misconfiguration cannot quietly turn the mechanism off.

`verified-by: bravebot_config::lib::the_context_budget_has_a_default`
`verified-by: bravebot_config::lib::the_context_budget_can_be_overridden`
`verified-by: bravebot_config::lib::a_budget_that_makes_no_sense_falls_back_rather_than_disabling_compaction`
`verified-by: bravebot_agent::turn::a_conversation_past_the_budget_is_summarised_before_the_next_request`
`verified-by: bravebot_agent::turn::compacting_on_request_reaches_the_model_and_shortens_the_conversation`

<a id="COMPACT-8"></a>
### COMPACT-8: compaction is tried before a request whose conversation is already over budget

The figure compared is what the server said the **last** round's request came to, so the check is
one round late by construction and the budget has to sit below the window rather than at it. A
turn that has not measured anything yet compacts nothing: there is no figure to compare, and a
guess would be one.

It is tried before every round that qualifies, not once per turn. Ordinarily there is nothing
worth cutting and nothing is sent, which costs nothing and says nothing. Only a summariser that
**failed** stops it being tried again for the rest of the turn, because a call that errored once
is not free to repeat.

`/compact` asks for the same work on demand, at any size, and does not consult the budget.

**Why the default sits well below any real window.** A budget above the window never fires, so
being wrong upward does not make compaction late, it removes it. The default was once set above
anything the backend serves and sessions ran to exhaustion having never been summarised. A default
is therefore checked against the smallest window worth using rather than chosen to be generous,
and anyone running a larger model raises it rather than discovering it was never doing anything.

`verified-by: bravebot_agent::turn::a_conversation_past_the_budget_is_summarised_before_the_next_request`
`verified-by: bravebot_agent::turn::a_conversation_nobody_has_measured_is_not_compacted`
`verified-by: bravebot_agent::conversation::a_long_turn_does_not_summarise_itself_once_per_round`
`verified-by: by-construction (the default budget is asserted below the smallest useful window at compile time)`
