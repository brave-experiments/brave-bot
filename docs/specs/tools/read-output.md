---
id: OUTPUT
title: read_output
status: normative
governs:
  - crates/core/src/policy.rs
guards:
  - symbol: Policy::read_output
---

## Scope

Letting the planner read what a program printed. The reference naming the result is routing; there
are no content arguments. What a program's output is labelled in the first place is
[run.md](run.md).

## Clauses

### OUTPUT-1: this is an assertion about bytes, and is not a relabel

A person is shown the bytes themselves with the command that printed them, and decides.

```
╭ let the model read this? ────────────────────────────────╮
│Read 1 line  printed by find /Applications -name 'Brave…' │
│                                                          │
│  the model has not seen this. Approving puts it in its   │
│  context, and it will act on it.                         │
│                                                          │
│┃ /Applications/Brave Browser Nightly.app                 │
│                                                          │
│  y let it read this    n keep it back    ctrl-c stop     │
╰──────────────────────────────────────────────────────────╯
```

The slot keeps the label it was quarantined at. What the planner receives is a **new value** whose
first label comes from the provenance the policy layer tracked, which is a person having read it. It
covers one result, needs a single-use endorsement naming that slot, and the next run asks again.

Only output from `run` can be read this way. A file's worth is the trust map's answer, and a second
route to it would be a way to disagree with the first.

**Why.** This is the strongest assertion in the system and the only one made about bytes rather
than about a prediction: vouching guesses at output that does not exist yet, while this is a
statement about text in front of the reader. Errors count: a run that failed put its explanation on
stderr, and a planner that cannot see it will report that the command worked.

`verified-by: bravebot_core::policy::output_a_person_vouched_for_comes_back_trusted`
`verified-by: bravebot_core::policy::output_a_person_vouched_for_is_still_private`
`verified-by: bravebot_core::policy::vouching_for_output_does_not_relabel_the_slot`
`verified-by: bravebot_core::policy::an_approval_to_read_output_cannot_be_replayed`
`verified-by: bravebot_agent::turn::output_a_person_reads_and_approves_reaches_the_planner`
`verified-by: bravebot_agent::turn::output_a_person_refuses_stays_out_of_the_planner`

### OUTPUT-2: only output from a program can be read this way

A file's worth is the trust map's answer, and naming a file, opening a directory and the startup
question already give it. A second route to the same decision would be a way to disagree with the
first.

`verified-by: bravebot_core::policy::a_file_cannot_be_promoted_by_reading_it_aloud`
