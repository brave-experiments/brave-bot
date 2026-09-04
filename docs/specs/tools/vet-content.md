---
id: VETC
title: vet_content
status: normative
governs:
  - crates/agent/src/tools.rs
---

## Scope

The call that asks whether one piece of quarantined content is safe to act on. `content_ref` is
routing; `expected` is content. The result is a verdict. What a checker is and what it may do is
[vetting.md](../vetting.md); this spec is the call surface.

## Clauses

<a id="VETC-1"></a>
### VETC-1: the call names one reference, and gets nothing else

One rather than a list. A reference naming nothing is refused.

`verified-by: bravebot_core::policy::only_the_slot_it_was_given_reaches_a_checker`
`verified-by: bravebot_core::policy::vetting_a_reference_to_nothing_is_refused`

<a id="VETC-2"></a>
### VETC-2: what the content was expected to be comes from the planner and may not be private

It is the only thing steering the question, and it comes from a context holding nothing an
attacker wrote. Leaving it out asks the narrower question rather than a laxer one.

`verified-by: bravebot_core::policy::a_private_expectation_cannot_direct_a_checker`
`verified-by: bravebot_agent::vet::a_checker_told_nothing_is_asked_the_narrower_question`

<a id="VETC-3"></a>
### VETC-3: the result is a verdict, and the content behind it stays where it was

The planner is told one of two things, in the driver's own sentence, and both of them say that
nothing about the reference has changed. The sentences the checker wrote go to the person watching
and no further.

`verified-by: bravebot_agent::turn::a_verdict_reaches_the_planner_and_what_it_was_about_does_not`
`verified-by: bravebot_agent::turn::a_clean_verdict_leaves_the_content_exactly_as_unreadable`
`verified-by: bravebot_agent::turn::a_checker_that_will_not_say_the_word_produces_an_unsafe_verdict`
