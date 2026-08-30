---
id: LABEL
title: Labels and who may read what
status: normative
governs:
  - crates/core/src/label.rs
  - crates/core/src/value.rs
  - crates/core/src/reference.rs
  - crates/core/src/slot.rs
guards:
  - symbol: Labelled::declassify
  - symbol: Labelled::relabel
  - symbol: Declassification::authorise
  - symbol: Policy::present
  - symbol: Policy::label_model_output
---

## Scope

What a label is, how one is assigned, which direction it may move, and which components may read
what carries one. Where an effect can land, meaning a write, a program run or a request out of this
process, is in [routing.md](routing.md). Which paths a person vouched for is in
[trust-map.md](trust-map.md).

## The lattice

<a id="LABEL-1"></a>
### LABEL-1: two axes, and a genuine lattice

```
L = I × C      I ∈ {T, U}      trusted / untrusted
               C ∈ {pub, priv} public  / private
```

Untrusted input degrades integrity; private input raises confidentiality. `(U,priv)` and `(T,pub)`
are **incomparable**, so this is a lattice rather than a pair of booleans and no total order may be
imposed on it. Integrity meets pessimistically and confidentiality joins pessimistically.

`verified-by: bravebot_core::label::middle_elements_are_incomparable`
`verified-by: bravebot_core::label::bottom_flows_everywhere_and_top_flows_nowhere`
`verified-by: bravebot_core::label::integrity_meet_is_pessimistic`
`verified-by: bravebot_core::label::confidentiality_join_is_pessimistic`
`verified-by: bravebot_core::label::ordering_is_reflexive_and_antisymmetric`
`verified-by: bravebot_core::label::ordering_is_transitive`
`verified-by: bravebot_core::label::display_matches_the_canonical_notation`

<a id="LABEL-2"></a>
### LABEL-2: a derived value is labelled by taint over its inputs

One untrusted input taints the result. One private input makes it private. The axes degrade
independently, the order of the inputs does not matter, and taint only ever degrades on each axis.
No inputs means no taint.

`verified-by: bravebot_core::label::taint_result_is_a_degradation_of_its_inputs`
`verified-by: bravebot_core::label::one_untrusted_input_taints_the_result`
`verified-by: bravebot_core::label::one_private_input_makes_the_result_private`
`verified-by: bravebot_core::label::axes_degrade_independently`
`verified-by: bravebot_core::label::taint_is_order_independent`
`verified-by: bravebot_core::label::taint_only_degrades_on_each_axis`
`verified-by: bravebot_core::label::no_inputs_means_no_taint`

## Who may read what

<a id="LABEL-3"></a>
### LABEL-3: nothing untrusted in the planner's context

Untrusted content is never placed in a message to the model. It is quarantined in a write-once
slot and the planner is given a **reference**: origin, line count, byte count, label. The planner
acts on content it cannot read by naming that reference, and the policy layer resolves it when the
write or the call actually happens. Where the content has to be changed rather than moved, it goes
to a processor, described in [processors.md](processors.md).

`verified-by: bravebot_core::policy::untrusted_content_is_presented_as_a_reference`
`verified-by: bravebot_core::policy::trusted_content_is_presented_visibly`
`verified-by: bravebot_core::reference::a_quarantined_presentation_shows_no_content`
`verified-by: bravebot_core::reference::a_visible_presentation_shows_the_content`
`verified-by: bravebot_core::reference::a_description_names_the_shape_and_not_the_content`
`verified-by: bravebot_core::reference::a_description_says_how_to_refer_to_the_content`

<a id="LABEL-4"></a>
### LABEL-4: nothing untrusted in the driver's context

The driver may **carry** a labelled value and hand it to an effect, but may not **read** one. The
type that carries a label offers no equality, no formatting, no dereference and no infallible
accessor, and its debug output redacts the value it holds. Reading requires a declassification
witness, which only the policy layer can mint: the part of `bravebot-core` that owns the gates, and
the only code here allowed to read untrusted bytes at all. Asking for them anywhere else returns a
refusal naming this rule, not a value.

**`bravebot-core` and `bravebot-agent` are both the driver.** Moving a branch from one
into the other does not remove it.

`verified-by: bravebot_core::value::untrusted_values_cannot_be_read_without_a_witness`
`verified-by: bravebot_core::value::trusted_public_values_need_no_witness`
`verified-by: bravebot_core::value::a_witness_permits_reading`
`verified-by: bravebot_core::value::debug_redacts_the_value`
`verified-by: by-construction (Deref, PartialEq and Display are not implemented for Labelled)`

<a id="LABEL-5"></a>
### LABEL-5: a decision may be taken only from trusted content

Comparing text is a decision. On trusted content that is fine, because a vouched-for path holds
nothing an attacker wrote. On untrusted content it is refused: the gate for reading content hands
over the bytes when they are trusted and **refuses** otherwise, so a caller cannot quietly take the
untrusted case. This is why editing a file requires a trusted one: locating a passage to replace
is a comparison.

Integrity is the only axis that matters here. Workspace content is private as a matter of course,
and examining it in-process releases nothing.

`verified-by: bravebot_core::policy::requesting_untrusted_content_is_refused`

<a id="LABEL-6"></a>
### LABEL-6: minting a witness is not permission to inspect

A witness records that bytes moved somewhere they were already allowed to go: a filesystem write,
an HTTP body, or a human's screen. Each of those three destinations has a gate of its own, one for
putting content in front of the planner, one for reshaping it for display, and one for reading
trusted content. A declassification anywhere else is almost certainly a violation.

`verified-by: none`

## Which direction a label may move

<a id="LABEL-7"></a>
### LABEL-7: labels only ever degrade

Integrity may go trusted to untrusted and never the reverse. Relabelling yields nothing rather than
upgrading, and refuses incomparable labels outright. Never build a labelled value by hand to give it
a better label than its inputs had: that is laundering, whichever crate it happens in. If a value
derived from untrusted input needs to be trusted for something to work, the
design is wrong, not the label.

`verified-by: bravebot_core::value::relabel_may_degrade`
`verified-by: bravebot_core::value::relabel_may_not_upgrade`
`verified-by: bravebot_core::value::relabel_refuses_incomparable_labels`
`verified-by: bravebot_core::label::degradation_is_not_the_lattice_ordering`
`verified-by: bravebot_core::label::trusted_input_cannot_launder_untrusted`
`verified-by: bravebot_core::label::top_of_taint_degrades_from_everything`

<a id="LABEL-8"></a>
### LABEL-8: a first label comes from provenance, and is not an upgrade

Model output is a function of the model's context and nothing else, so when the context holds only
trusted input, what it produced is labelled accordingly. The same road labels a program's output, a
line the user ran themselves, the user's own configuration and a pasted picture. Each is the
**first** label such a value ever receives, assigned from provenance the
policy layer tracked.

If you find yourself relabelling a value that already has a label, stop: that is LABEL-7.

`verified-by: bravebot_core::policy::model_output_from_a_clean_context_is_trusted`
`verified-by: bravebot_core::policy::observation_labels_come_from_the_capability`

<a id="LABEL-9"></a>
### LABEL-9: context integrity falls when the planner is shown something, never when a turn reads it

Context integrity only ever falls, and it falls when content is put in front of the planner. A
quarantined read
puts a reference in the context, not the bytes, and a slot id with a line count carries no
instruction. A paste does not lower it either, and resuming cannot raise it.

**Why.** Lowering integrity at the observation would label the planner's own words untrusted on the
strength of a file it never saw, and `present` would then quarantine the planner from itself.
Never move this back to the observation.

**This cannot happen today.** A context only becomes untrusted by resuming one that already was,
and nothing makes one untrusted in the first place: untrusted content is quarantined rather than
shown, and the only place the context absorbs anything is where content was trusted enough to show.
The gate is here anyway, because what makes that closure safe to rely on is that something refuses
if it ever stops holding. If a change ever lets untrusted bytes into the planner's context, this is
what catches it.

`verified-by: bravebot_core::policy::context_integrity_never_recovers`
`verified-by: bravebot_core::policy::a_quarantined_read_leaves_the_planner_able_to_see_its_own_words`
`verified-by: bravebot_core::policy::a_pasted_image_does_not_lower_what_the_context_has_met`
`verified-by: bravebot_core::policy::resuming_cannot_raise_the_integrity_of_a_context`
`verified-by: bravebot_core::policy::answering_never_raises_a_context_that_has_already_fallen`
`verified-by: bravebot_agent::turn::what_the_planner_writes_after_a_quarantined_read_stays_trusted`

## Known costs

- **Two places in the policy layer do look at untrusted bytes in order to decide something.** The
  clauses above say nothing may, so these are exceptions, and they are written down rather than
  left to be found.

  The first is splitting a processor's answer. A processor hands back a single piece of text that
  holds two things: a remark meant for the person watching, and the document to be written. It
  marks the line where the document starts, and the policy layer searches the text for that mark to
  find where to cut. Searching text is a decision, and this text came from a processor, so it is
  untrusted. The second is smaller: before a file is written back, the code checks whether the file
  being replaced ended in a newline, so the new one can end the same way.

  The mark is not a boundary and cannot be forged, because there is nothing to forge: the processor
  writes the whole answer, so it is entitled to put the mark wherever it likes, including more than
  once, and the first one is the one that counts. It is a declaration, not a guarantee.

  What makes that acceptable is worth spelling out, because "it looks at untrusted bytes" is
  exactly the shape of a real hole. Suppose an attacker owns the file, so they steer what the
  processor writes and where it puts the mark. Everything that buys them is on this list:

  - **Leave the mark out.** Then there is no document and the write is refused. They stopped
    something from happening, which is the direction this is built to fail in.
  - **Put the mark somewhere else, or more than once.** That shifts where the text is cut, so
    their words land in the document rather than in the remark. But the document becomes the body
    of a file the planner named and a person approved from a diff, and its contents were already
    coming from the attacker's file. They gain nothing they did not already have.
  - **Put their words in the remark.** It reaches a person's screen and stops there. It is drawn as
    untrusted content, inside a margin it cannot forge, and no model is given it. It can still
    *lie*: the remark is free text attributed to the processor, so it can claim the document only
    fixes a typo when it does something else. Nothing checks a remark against the document it
    accompanies, and nothing could. What keeps that from mattering is that the remark is not the
    decision. The write is approved later, from a diff of the actual bytes, so a person who reads
    the diff sees what happens whatever the remark said. The residue is that a plausible remark
    might persuade somebody to skim, which is
    [issue #23](https://github.com/brave-experiments/brave-bot/issues/23).
  - **Add or drop a trailing newline.**

  What is not on the list is the thing that would matter: choosing *which* file is written. That
  stays the planner's choice plus a person's approval, and no amount of steering the text changes
  it. The clauses above forbid decisions that redirect an effect, and none of these redirect
  anything.
