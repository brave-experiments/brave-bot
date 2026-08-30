---
id: ASK
title: ask_user
status: normative
governs:
  - crates/core/src/ask.rs
  - crates/tui/src/ask.rs
guards:
  - symbol: Policy::record_answers
---

## Scope

`questions` is routing; there are no content arguments. The result is what the user answered,
question by question. This is the one tool whose result comes from a person rather than from the
workspace, and the
only one with no effect at all.

## Clauses

### ASK-1: the questions are routing, approved by being read

`ask_user` still has a destination, the user's screen, and the questions and their options decide
what the user is shown and therefore what they can answer. What is drawn is exactly the bytes the
gate checked; nothing re-parses them afterwards, and there is no effect to endorse beyond the
display.

`verified-by: bravebot_core::ask::a_shaped_question_carries_the_tag_it_is_shown_under`
`verified-by: bravebot_core::ask::a_question_with_no_options_is_still_a_question`

### ASK-2: one to four questions, refused whole rather than trimmed

One call carries one to four questions. More is refused, never trimmed. The gate runs **once**,
over a string covering every question in the call.

**Why.** Checking them one at a time would mean deciding, per question, whether that one is put to
the user, and which half of a set survives would then be a decision taken from what is in it. A
question quietly dropped would be one the planner was told the user had been asked and they never
saw.

`verified-by: bravebot_core::ask::a_series_key_covers_every_question_in_it`
`verified-by: bravebot_core::ask::a_series_key_says_how_many_questions_are_in_it`
`verified-by: bravebot_core::ask::shaping_a_series_never_drops_a_question`
`verified-by: bravebot_core::policy::a_refused_series_yields_no_answer_at_all`

### ASK-3: asking stops once the planner's context has met something untrusted

The questions carry the integrity of the planner's context. Once that context is untrusted, the
routing gate refuses them and the planner is told so and continues without an answer.

**Why.** A question shown to the user may have been written from bytes an attacker controlled, and
choosing among strings an attacker wrote does not make those strings trustworthy. Treating the
keypress as though it did would carry injected text into the planner's context.

**This cannot happen today.** A context only becomes untrusted by resuming one that already was,
and nothing makes one untrusted in the first place: untrusted content is quarantined rather than
shown, and the only place the context absorbs anything is where content was trusted enough to show.
The gate is here anyway, because what makes that closure safe to rely on is that something refuses
if it ever stops holding. If a change ever lets untrusted bytes into the planner's context, this is
what catches it.

`verified-by: bravebot_core::policy::a_series_from_an_untrusted_context_cannot_be_put_to_the_user`
`verified-by: bravebot_core::policy::answers_to_an_untrusted_series_are_refused`

### ASK-4: a quarantined read does not stop the planner asking

Context integrity falls when the planner is **shown** something untrusted, never when a turn reads
something. A quarantined read hands the planner a reference and never the bytes, so nothing in
that file could have shaped the question.

**Why.** Lowering integrity at the read would label the planner's own words untrusted on the
strength of a file it never saw, and presentation would then quarantine the planner from itself.
Never move this back to the observation.

`verified-by: bravebot_agent::turn::a_quarantined_read_does_not_stop_the_planner_asking`
`verified-by: bravebot_agent::turn::what_the_planner_writes_after_a_quarantined_read_stays_trusted`
`verified-by: bravebot_core::policy::a_quarantined_read_leaves_the_planner_able_to_see_its_own_words`

### ASK-5: an answer is trusted as a first label, and only for a trustworthy question

The bytes came from the user's keyboard, the same source as the task itself. That is a first label
from provenance, not an upgrade. It is still refused when the question being answered was not
itself trustworthy (ASK-3).

`verified-by: bravebot_agent::turn::every_answer_in_a_series_reaches_the_planner`

### ASK-6: skipping answers a question, and questions are put one at a time

Questions are put one at a time with the position shown. For each, the user may pick an option,
pick several where allowed, answer in their own words, or skip. Skipping moves to the next
question rather than abandoning the rest, and the planner is told, question by question, what was
answered and what was passed over.

`verified-by: bravebot_core::ask::a_skipped_question_is_reported_as_declined_beside_its_answered_siblings`
`verified-by: bravebot_core::ask::a_series_is_reported_question_by_question`
`verified-by: bravebot_core::policy::declining_is_an_answer_rather_than_a_refusal`
`verified-by: bravebot_core::policy::fewer_answers_than_questions_are_read_as_declines`
`verified-by: bravebot_core::policy::more_answers_than_questions_are_dropped`
`verified-by: bravebot_core::ask::an_answer_with_no_question_is_dropped_rather_than_attributed_to_one`
`verified-by: bravebot_agent::turn::a_skipped_question_comes_back_as_a_decline_beside_the_rest`

### ASK-7: where nobody can be asked, every question is declined

In a one-shot `bravebot "..."` run, every question in the series is declined rather than answered
on the user's behalf.

**Why.** The planner is told the reply came from a person, so inventing one would be worse than
not asking.

`verified-by: bravebot_agent::turn::an_unattended_run_declines_every_question_in_the_series`

### ASK-8: an answer is remembered for the session, question by question

A planner that loops back over the same decision does not make the user restate it, and a set
where some are already settled shows only the rest. Questions differing anywhere, including in
their tag, are different questions.

`verified-by: bravebot_core::ask::the_key_distinguishes_questions_that_differ_anywhere`
`verified-by: bravebot_core::ask::the_key_distinguishes_questions_that_differ_in_their_tag`
`verified-by: bravebot_core::ask::the_key_is_stable_for_the_same_question`
`verified-by: bravebot_core::ask::two_series_holding_the_same_questions_in_a_different_order_have_different_keys`
`verified-by: bravebot_tui::ask::a_series_answered_entirely_from_memory_asks_nothing`
`verified-by: bravebot_tui::ask::remembered_answers_keep_their_places_beside_the_fresh_ones`
