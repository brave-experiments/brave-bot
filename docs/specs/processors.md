---
id: PROC
title: Processors
status: normative
governs:
  - crates/core/src/processor.rs
  - crates/core/src/policy.rs
guards:
  - symbol: Policy::before_processor
  - symbol: Policy::compose_processor_input
  - symbol: Policy::write_belongs_here
  - symbol: Policy::declassify_into_workspace
---

## Scope

The one component in the system that reads untrusted content. What it is given, what it may do,
and how what it produces is labelled and written.

## Why it exists

An agent that may not read a file also cannot change it: `edit_file` refuses on an untrusted file
and a whole-file `write_file` would need a body the planner could only have guessed.
Without processors the agent could answer questions about a repository nobody vouched for and do
no work in one.

## Clauses

<a id="PROC-1"></a>
### PROC-1: a processor holds no capabilities at all

| | |
|---|---|
| Tools | none, and the request carries no tool list at all |
| Memory | none: the messages are built from nothing each time |
| Conversation | one request, one reply, no loop to steer |
| Reads | exactly the references named in `reads`, and nothing else |
| Writes | at most one new reference, and nothing else |

A call mints **one slot or none**: the document, when the answer said where the document begins,
and nothing at all when it did not. The remark is a second output but never a slot: it is reported
to the person watching and cannot be read, written or passed to another processor.

**Why.** A processor with one tool is a second planner with untrusted content in its context,
which is the thing this design refuses. A loop would give a reply something to steer.

`verified-by: bravebot_core::policy::only_the_slots_it_was_given_reach_a_processor`
`verified-by: bravebot_agent::turn::a_tool_call_from_a_processor_does_nothing`
`verified-by: bravebot_core::policy::a_reference_to_nothing_is_refused`
`verified-by: bravebot_core::policy::a_processor_with_nothing_to_read_is_refused`
`verified-by: bravebot_core::policy::naming_the_same_reference_twice_is_refused`

<a id="PROC-2"></a>
### PROC-2: the spec is built by the driver and frozen before the run

The driver builds the spec, in one place, and nothing widens one afterwards.

`verified-by: bravebot_core::processor::a_description_names_the_slots_and_the_label_but_no_content`

<a id="PROC-3"></a>
### PROC-3: the output label is computed before the processor runs

By taint over the inputs. Nothing the processor writes has any say in how what it writes is
labelled.

`verified-by: bravebot_core::policy::an_output_is_labelled_by_taint_over_the_inputs`

<a id="PROC-4"></a>
### PROC-4: the input is assembled by the policy layer

The policy layer, the part of `bravebot-core` that owns the gates, concatenates the slots. The driver carries the result wrapped and hands it to the call
without seeing it. The `instruction` comes from the planner and must not
be private.

`verified-by: bravebot_core::policy::a_private_instruction_cannot_direct_a_processor`

<a id="PROC-5"></a>
### PROC-5: the output is never shown to the planner

It is presented like any other untrusted content: a reference, and nothing else.

`verified-by: bravebot_agent::turn::the_planner_is_told_the_shape_of_what_a_processor_produced`
`verified-by: bravebot_agent::turn::what_a_processor_says_reaches_the_person_and_no_model`

<a id="PROC-6"></a>
### PROC-6: an answer is for one document, and belongs only where the planner said

A write of a processor's answer is refused anywhere but the file the planner said the call was
about. Where the planner said nothing and there was more than one
input, the answer belongs nowhere and may be written nowhere.

**Why.** This is not a label rule and cannot be one. Every gate passed when a planner wrote a
game's HTML into a Python script, because the destination was a path it named and nothing about
the labels said otherwise.

`verified-by: none`

<a id="PROC-7"></a>
### PROC-7: nothing a processor writes is a file unless it says where the file begins

Everything before the document marker is a remark for the person watching: it reaches a
screen and stops, no model reads it, it is not part of any file, and it cannot be another
processor's input. Everything after the line is the document. An answer with no line names no
document and can be written nowhere.

**Why.** That way round on purpose. It was the other way, prose being the default and the line the
exception, and a processor explaining why it was leaving a Python script alone wrote the
explanation over the script. A processor has one output and has always wanted two, so forgetting
which is which has to fail towards changing nothing. Which document the answer is for is likewise
marked on the document, not left to the instruction to describe: one given two files and told in
prose the answer was for the second returned the first, and the first went into the second's file.

`verified-by: bravebot_core::policy::an_answer_without_the_line_names_no_document`
`verified-by: bravebot_core::policy::the_document_to_return_is_marked_on_the_document`
`verified-by: bravebot_core::policy::what_a_processor_says_is_split_from_what_it_produced`
`verified-by: bravebot_core::policy::a_processor_can_say_why_it_left_a_document_alone`

<a id="PROC-8"></a>
### PROC-8: quarantined content becomes a file body, and never anything else

A private slot may become a file body. That is sound only
because the destination is inside the boundary the bytes came from, so nothing leaves. The trust map
then records that path as untrusted, so reading it back does not launder it.

**Never** reach for it for a network body, a command line, or a message to someone.

`verified-by: bravebot_core::policy::a_write_back_into_the_workspace_lowers_only_confidentiality`

## Boundaries

<a id="PROC-9"></a>
### PROC-9: the confinement is the capability set, not an operating system boundary

There is no untrusted code involved: the call is made by the same trusted driver that makes every
other call. `bravebot-sandbox` confines processes running code we did not write, and putting a
processor in a subprocess would confine the wrong thing.

`verified-by: none`

## Known costs

- **An untrusted file's contents reach the backend.** A processor is a model call, so working on
  a file nobody vouched for sends it where before it would have stayed on the machine. The
  destination is the one the planner's own context has always gone to. What is new is only that
  the reader holds nothing.
- **One approval per candidate, not per change.** Where several files could be the one, each is
  read into its own slot, transformed with the same instruction, and written back to the path it
  came from. A file the processor left alone is written back byte for byte, and still costs an
  approval.
