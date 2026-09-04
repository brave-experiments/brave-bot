---
id: TRACE
title: The trace
status: normative
governs:
  - crates/core/src/event.rs
  - crates/tui/src/audit.rs
---

## Scope

What is recorded about every decision the system makes, what that record may contain, and how it
is read back.

A **gate** is a check that has to pass before anything consequential happens: content reaching the
model, a file being written, a program being run, a request leaving the process. Each one decides a
single question and refuses rather than warning, so there is no path to a consequence that does not
go through one. The trail is the record of those decisions.

## Clauses

<a id="TRACE-1"></a>
### TRACE-1: every gate decision is recorded, allowed or refused

A refusal is as much a record as a permission. A read and a write leave different trails, a
promotion is recorded as one, and the fields fixed before a turn observed anything are recorded
first.

**Why.** A trail that logged only what happened would not answer "why did it not do the thing I
asked", which is most of what anyone asks it.

`verified-by: bravebot_core::policy::a_read_and_a_write_leave_different_trails`
`verified-by: bravebot_core::policy::promotion_appears_in_the_audit_trail`
`verified-by: bravebot_core::policy::the_audit_trail_records_the_precommit_first`
`verified-by: bravebot_core::policy::a_turn_cannot_begin_without_routing`

<a id="TRACE-2"></a>
### TRACE-2: the trail holds no content

Every field is a gate name, a capability, a label, a path or a slot id. That is why it can be shown
on a screen and written to a file without any release, and it is what makes the record safe to keep
for a workspace nobody vouched for.

`verified-by: none`

<a id="TRACE-3"></a>
### TRACE-3: an assertion a person made is recorded as one

Vouching for the output of a command the user typed, labelling their configuration, and admitting a
pasted picture are each written down, because each is a claim a human made rather than something
the system worked out.

**Why.** These are the points where trust enters from outside. A trail that recorded only what the
system deduced would omit exactly the decisions somebody might later want to account for.

`verified-by: bravebot_core::policy::trusting_a_typed_commands_output_is_recorded_in_the_audit_trail`
`verified-by: bravebot_core::policy::labelling_configuration_is_recorded_in_the_audit_trail`
`verified-by: bravebot_core::policy::a_pasted_image_is_recorded_in_the_audit_trail`

<a id="TRACE-4"></a>
### TRACE-4: on disk it is one JSON object per line, with the axes written out in words

A session's trail is appended a turn at a time, so a line-oriented file can be read with whatever
is to hand. The labels are spelled out rather than abbreviated, because a file read months later
has no legend beside it. Each event keeps the time it happened.

**Why.** The compact form suits a terminal, where the reader has the legend in front of them. A
file has a different reader.

`verified-by: bravebot_tui::sessions::the_audit_keeps_the_time_each_event_happened`

A **gate** is a check that has to pass before anything consequential happens: content reaching the
model, a file being written, a program being run, a request leaving the process. Each one decides a
single question and refuses rather than warning, so there is no path to a consequence that does not
go through one. Every decision a gate makes is recorded, and the blocks below are those records.

Three pieces of notation appear in them. `(T,pub)` and `(U,priv)` are the label on a value: trusted
or untrusted on the first axis, public or private on the second. `ref:N` is a slot holding content
the planner is not allowed to read, so it is handed the reference instead of the bytes. And
`routing` marks the part of a call that decides where it lands, as opposed to the part that is
merely carried.

Reading a file a checker has already cleared, where the content reaches the model:

```
ok      precommit: routing fields ["task"] fixed before any observation
ok      promote: read_file.path proposed by the model, confined and non-destructive
ok      file_read.path [routing] (T,pub)
observe file_read produced (T,priv)
ok      trust: notes.md read as trusted, from a trusted path
ok      render: read_file: content reshaped for presentation, still (T,priv)
ok      present: tool_result: notes.md is (T,priv), so the planner may read it
```

The same read where nothing is vouched for, so the content is quarantined instead:

```
observe file_read produced (U,priv)
ok      trust: notes.md read as untrusted
slot    ref:0 at (U,priv)
ok      present: tool_result: notes.md is (U,priv), quarantined as ref:0; the planner
        sees a reference only
```

Changing that same file, which nothing along the way is able to read:

```
ok      reference: spawn_processor.reads names ref:1
ok      processor: processor over ref:1 reads ref:1 and writes (U,priv), with no tools,
        no memory and nothing to write but that one slot
ok      processor: input assembled from 1 slot(s) inside the kernel
ok      processor: output labelled (U,priv) by taint over its inputs
slot    ref:3 at (U,priv)
ok      present: tool_result: quarantined as ref:3; the planner sees a reference only
ok      resolve: write_file: ref:3 resolved to its quarantined content, (U,priv)
release ref:3 (U,priv) -> (U,pub)
ok      declassify: ref:3 released into src/config.py, which is inside the workspace
ok      approval: src/config.py: a path nobody has vouched for either way, asking
```

<a id="TRACE-5"></a>
### TRACE-5: the trail is readable live and after the fact

`--trace` on a one-shot run prints the same thing, and Ctrl-T toggles it in a session. Each line is
one gate that ran: what it checked, the label it saw, and what it allowed. It is the fastest way to
find out why something was refused.

`verified-by: bravebot_tui::app::ctrl_t_toggles_the_trail`

<a id="TRACE-6"></a>
### TRACE-6: each planning call is recorded, like any other gate

A manifest run makes two of them, and both appear in the trail: one for the goal in plain words,
and one for fitting that to the tool set. A refusal is as much a record as a permission.

`verified-by: bravebot_agent::manifest::the_audit_trail_records_each_planning_call`
