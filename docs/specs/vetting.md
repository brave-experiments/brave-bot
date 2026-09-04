---
id: VET
title: Vetting quarantined content
status: normative
governs:
  - crates/core/src/vet.rs
  - crates/agent/src/vet.rs
guards:
  - symbol: Policy::before_vet
  - symbol: Policy::compose_vet_input
  - symbol: Policy::read_verdict
---

## Scope

Asking whether one piece of quarantined content is what it was supposed to be, and whether it
tries to direct whoever reads it. What the question is, who may ask it, and what an answer is
allowed to reach. The call surface is [tools/vet-content.md](tools/vet-content.md).

## Why it exists

Everything else in this system arranges for untrusted content to be moved without being judged.
That is enough to work on a file nobody vouched for and not enough to decide whether to. A planner
that fetched a page and wants to know whether the page is a page, before it spends a turn treating
it as one, has no way to find out: reading it is the thing it may not do, and the reference says
how many lines there are and nothing about what is in them.

A vetting call answers that one question and no other. It is worth having because the answer is
cheap, and worth being careful about because it is the only answer of its kind: for one bit, per
call, what untrusted content reaches is the planner rather than a slot.

## Clauses

<a id="VET-1"></a>
### VET-1: a checker holds no capabilities at all

| | |
|---|---|
| Tools | none, and the request carries no tool list at all |
| Memory | none: the messages are built from nothing each time |
| Conversation | one request, one reply, no loop to steer |
| Reads | exactly the content the call was built for, and nothing else |
| Writes | nothing at all: no slot, no file, no reference |

One thing rather than several, because a verdict is about a thing: a call given two documents and
answering in one word has said something about neither. What that thing is, a reference or a file,
is the driver's own name for it and is routing either way.

**Why.** A checker with one tool is a second planner with untrusted content in its context, which
is the thing this design refuses. A loop would give a reply something to steer.

`verified-by: bravebot_core::policy::only_the_content_it_was_given_reaches_a_checker`
`verified-by: bravebot_core::policy::content_at_another_label_than_the_spec_fixed_is_refused`

<a id="VET-2"></a>
### VET-2: the question is fixed by the driver before the content is read

What is being vetted, what it was expected to be, and the label of what is read are all settled in
one place before any byte is touched, and nothing widens the question afterwards. Content at any
other label than the one fixed is refused rather than vetted, so the label a verdict is about is
always the label somebody fixed. The expectation comes from the planner and must not be private.

`verified-by: bravebot_core::policy::a_private_expectation_cannot_direct_a_checker`
`verified-by: bravebot_core::vet::a_description_names_the_subject_and_the_label_but_no_content`

<a id="VET-3"></a>
### VET-3: silence about what to expect narrows the question rather than widening it

Where the planner said what the content was supposed to be, content that is not that thing fails
even if it asks for nothing. Where the planner said nothing, the question is only whether the
content tries to direct its reader.

**Why.** A planner is not obliged to know what it fetched, and a checker told nothing must not
read that as permission. The half of the question that can always be asked is asked either way.

`verified-by: bravebot_agent::vet::a_checker_is_told_what_the_content_was_expected_to_be`
`verified-by: bravebot_agent::vet::a_checker_told_nothing_is_asked_the_narrower_question`

<a id="VET-4"></a>
### VET-4: the verdict is one of two words the driver wrote, and nothing else is a pass

The driver searches the reply for two literals of its own and reports one of two states. An answer
holding neither word, or both, is the failing one. Nothing a checker writes reaches the planner
verbatim.

**Why.** The reply is tainted by what it read, so what comes out of it has to be something the
driver chose rather than something the content wrote. Failing towards the word that tells the
planner less is the direction a confused checker, a truncated reply, or a talked-round one can
only fail in.

`verified-by: bravebot_core::policy::a_checker_that_says_the_word_is_believed`
`verified-by: bravebot_core::policy::an_answer_with_neither_word_is_unsafe`
`verified-by: bravebot_core::policy::an_answer_with_both_words_is_unsafe`
`verified-by: bravebot_agent::vet::the_prompt_asks_for_one_of_the_two_words_the_kernel_reads`

<a id="VET-5"></a>
### VET-5: why the checker said so reaches a person and no model

The sentences behind a verdict are presented like any other quarantined content: to the screen,
inside a margin they cannot forge, with no model reading them. They are not part of any file and
cannot be another call's input.

`verified-by: bravebot_core::policy::what_a_checker_said_cannot_come_back_better_than_what_went_in`
`verified-by: bravebot_core::policy::what_a_checker_said_is_labelled_by_taint_over_the_input`

<a id="VET-6"></a>
### VET-6: a verdict never relabels anything, and `vet_content` grants nothing at all

A verdict is not written onto the value it was about. Nothing carries a label upward, and the
answer a planner gets from asking about a reference leaves that reference exactly as unreadable as
it was.

Where a verdict does decide something, it decides it the way a person's approval of a program's
output does: by establishing a **new** label from provenance the policy layer tracked, never by
raising an old one. There is one such place, the read of a file nobody vouched for, and it is
[trust-map.md](trust-map.md).

**Why.** A label is provenance and a verdict is an opinion about bytes. Writing the second over the
first would be laundering; recording that a check happened, and labelling what comes afterwards
from that record, is the same move every other grant in the trust map makes.

`verified-by: bravebot_agent::turn::a_clean_verdict_leaves_the_content_exactly_as_unreadable`
`verified-by: bravebot_agent::turn::a_file_a_checker_will_not_clear_stays_quarantined`

## Known costs

- **One bit per call reaches the planner's context from untrusted content.** Everywhere else in
  this system, what untrusted bytes reach is a slot the planner cannot read. Here they reach a
  verdict, and a verdict nobody is told is not a verdict. This is a deliberate exception and it is
  written here rather than left to be found.

  The channel is one of two words per call. It is not free text, it is not the checker's own
  wording, and the sentences behind it never leave the screen. But an attacker who owns the
  content owns what the checker concludes about it, so they own the bit: they can make a page that
  reads as harmless and get a pass, or make one that reads as an attack and get a failure. Both of
  those are the classifier being what it is.

  What that buys them is bounded by what a verdict does, which today is nothing. It changes no
  label, moves no file and opens no slot. The planner is told a word and decides what to make of
  it, from a context that still holds none of the content. Signalling across several calls is
  possible in principle, since the planner chooses when to call and the content can vary its
  answer, but the planner chooses, not the content, and what arrives is still one word at a time.

  The cost that would matter is a verdict becoming load-bearing: something that treats a pass as
  grounds to trust the bytes. That is not this spec, and a change proposing it is proposing to let
  a model reading attacker-controlled text decide a label.

- **The driver reads the reply to find the verdict.** Searching text is a decision and the reply
  is untrusted, so this is one of the deliberate exceptions listed in
  [labels.md](labels.md). What the search is for is text the driver wrote, what it produces is one
  of two words the driver wrote, and an answer matching neither fails.

- **A vetting call sends the content to the backend.** A checker is a model call, so vetting a
  file nobody vouched for sends it where before it would have stayed on the machine. The
  destination is the one the planner's own context already goes to. What is new is only that these
  bytes go there without the planner or the driver reading them.
