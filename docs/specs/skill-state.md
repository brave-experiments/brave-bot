---
id: STATE
title: A run that carries a state instead of a history
status: normative
governs:
  - crates/core/src/state.rs
  - crates/agent/src/state.rs
guards:
  - symbol: Policy::adopt_state_patch
  - symbol: State::merged
---

## Scope

The third way a task is run. A turn re-sends the whole conversation every round, so its request
grows with the length of the run and [compaction.md](compaction.md) exists to cut it back down. This
mode never sends the history at all. Each step carries the task, one structured execution state the
model maintains itself, and the newest observation.

The default remains the turn loop. This is not a stricter mode and not a safer one: the gates are
the same gates and an action is the same action. What changes is what the model is shown in order to
choose one.

## What it trades

A bounded request, against everything the model failed to write down. The state has to be a
sufficient statistic for the rest of the run, and there are tasks where it cannot be: the object of
"what did you change, and why" is the trajectory itself, and this is the mode that stopped sending
the trajectory. That is a reason to choose the mode deliberately rather than a defect to fix, and it
is why the default is the other one.

## Why a patch is checked the way a summary is

A patch is model output going back into the model's own context, which is what a compaction summary
is. Both are therefore refused once the context has gone untrusted, and for the same reason: there
is nowhere else for them to go. Quarantining a patch would hand the model a reference in place of
the state it decides from, which is not a state it can decide from, and relabelling it would be
laundering.

So the state holds whatever the model may already say out loud, which includes the *name* of a
quarantined thing and never a byte of what is behind it.

## Clauses

<a id="STATE-1"></a>
### STATE-1: a step is shown the task, the state, and the newest observation, and nothing else

The request is assembled from those four things afresh every step: the instructions, the task, the
rendered state, and at most one observation. It is never appended to. A run of two hundred steps
sends a request the size of a run of one.

**This is the whole claim of the mode**, so it is stated over what goes on the wire rather than over
any figure the driver keeps. A driver that believed itself bounded while appending anyway would
satisfy every other clause here.

An observation from an earlier step is gone. Not summarised, not truncated: absent.

`verified-by: bravebot_agent::state::the_request_does_not_grow_with_the_number_of_steps`
`verified-by: bravebot_agent::state::a_first_step_with_nothing_observed_sends_no_observation`
`verified-by: bravebot_agent::state::every_step_sends_the_same_number_of_messages`
`verified-by: bravebot_agent::state::a_later_step_is_not_larger_than_an_earlier_one`
`verified-by: bravebot_agent::state::an_earlier_observation_is_not_sent_again`

<a id="STATE-2"></a>
### STATE-2: a key the patch does not mention keeps its value

A patch names the keys that changed. Setting a key sets it, a null deletes it, and an object merges
into the object already there key by key, so a patch about one field leaves every sibling of it
alone. There is no way to express "and the rest is gone".

**Why.** This is the failure mode the design is most exposed to, measured as 68% of all errors on
smaller models: the state overwritten rather than merged. A shape that cannot express a wholesale
overwrite of untouched keys cannot suffer it, so forgetting to repeat a key does not lose it.

A group emptied of everything is dropped, since a key that can no longer say anything is a key the
model reads every step for nothing.

`verified-by: bravebot_core::state::a_key_the_patch_does_not_mention_survives_it`
`verified-by: bravebot_core::state::a_patch_into_a_group_leaves_the_rest_of_the_group_alone`
`verified-by: bravebot_core::state::deleting_a_key_inside_a_group_removes_only_that_key`
`verified-by: bravebot_core::state::merging_into_a_group_that_does_not_exist_yet_creates_it`
`verified-by: bravebot_core::state::deleting_a_key_removes_it`
`verified-by: bravebot_core::state::a_group_emptied_of_everything_is_pruned`
`verified-by: bravebot_core::state::a_list_is_replaced_rather_than_merged`
`verified-by: bravebot_agent::state::a_later_patch_does_not_drop_what_an_earlier_one_recorded`
`verified-by: bravebot_agent::state::what_a_step_recorded_reaches_every_later_step`

<a id="STATE-3"></a>
### STATE-3: the merge is pure and total, and a refused patch changes nothing

The merge sees a state and a patch and nothing else, decides the same way every time, and either
produces a whole new state or leaves the old one exactly as it was. No state is half merged and no
key is repaired to make a patch fit.

`verified-by: bravebot_core::state::a_refused_patch_changes_nothing`
`verified-by: bravebot_core::state::rendering_is_deterministic_whatever_order_keys_arrived_in`
`verified-by: bravebot_core::state::setting_a_key_outright_replaces_whatever_shape_was_there`
`verified-by: bravebot_core::state::merging_a_group_into_a_single_value_is_refused`

<a id="STATE-4"></a>
### STATE-4: the state has a size, and reaching it refuses the patch rather than trimming it

Depth, the width of any one group, and the size of the rendered whole are all bounded, and the
bounds are checked on the result rather than on the patch. Passing one refuses the patch and says
what would fix it.

**Why a refusal and not a trim.** The bound is the point of the mode, so a state allowed to creep
past it gives back the growing request the mode exists to remove. A runtime that silently dropped
the oldest key to make room would be choosing what the model forgets, badly, and without saying so.
The model is the only party that knows which of its notes it still needs, so it is the one asked.

An empty key is refused, at any depth and inside a list: a key nothing can name holds something no
later patch can update.

`verified-by: bravebot_core::state::a_state_over_the_byte_budget_is_refused`
`verified-by: bravebot_core::state::a_small_patch_that_would_overfill_the_state_is_refused`
`verified-by: bravebot_core::state::a_state_nested_deeper_than_the_limit_is_refused`
`verified-by: bravebot_core::state::a_group_wider_than_the_limit_is_refused`
`verified-by: bravebot_core::state::an_oversized_state_says_what_would_fix_it`
`verified-by: bravebot_core::state::an_empty_key_is_refused`
`verified-by: bravebot_core::state::an_empty_key_inside_a_value_is_refused_at_any_depth`

<a id="STATE-5"></a>
### STATE-5: a patch is adopted only while the context is trusted

Once the context has gone untrusted the patch is refused and the run keeps the state it had. It is
not quarantined instead, and it is not relabelled to get past the gate.

**This cannot happen today**, for the reason the same clause in [compaction.md](compaction.md)
cannot: untrusted content is quarantined rather than shown, so the context only becomes untrusted by
resuming one that already was. The gate is here anyway, because what makes that closure safe to rely
on is that something refuses if it ever stops holding.

`verified-by: bravebot_core::policy::a_patch_from_a_trusted_context_is_adopted`
`verified-by: bravebot_core::policy::a_patch_from_an_untrusted_context_is_refused_rather_than_adopted`

<a id="STATE-6"></a>
### STATE-6: the state is rendered by the kernel, and holds a closed set of values

Text, whole numbers, booleans, lists and named groups, and nothing else. The rendering escapes what
JSON requires, and there is no second renderer for a caller to reach for.

**Why the kernel owns it.** What goes into the request is this structure written out, and a key or a
value holding a quote mark, written as it stands, would close the string it was in and put structure
into the prompt that no state ever held. A value shape with no case is refused rather than coerced,
because a runtime that quietly turned a fraction into something else would leave a state disagreeing
with what the model believes it recorded.

`verified-by: bravebot_core::state::a_key_or_value_holding_json_punctuation_cannot_add_structure`
`verified-by: bravebot_core::state::a_newline_in_a_value_does_not_break_the_line`
`verified-by: bravebot_core::state::a_new_state_is_empty_and_renders_as_an_object`
`verified-by: bravebot_agent::state::the_state_reaches_the_model_through_the_kernels_renderer`
`verified-by: bravebot_agent::state::a_fractional_number_is_refused_rather_than_rounded`
`verified-by: bravebot_agent::state::a_list_is_read_as_a_list`
`verified-by: bravebot_agent::state::a_patch_of_plain_values_is_read`
`verified-by: bravebot_agent::state::a_null_deletes_the_key`
`verified-by: bravebot_agent::state::a_nested_null_deletes_only_that_key`
`verified-by: bravebot_agent::state::an_object_in_a_patch_merges_rather_than_replacing`

<a id="STATE-7"></a>
### STATE-7: the patch arrives in a tool call, and a bad one does not end the run

The patch is a field of a call rather than a block to be found in prose, so there is nothing to
search for and no way to pick the wrong document. A patch that will not decode, or that the merge
refuses, comes back to the model as its next observation, in words saying what was wrong.

**Why it does not end the run.** Every way a patch can fail is something the model can do
differently on the next step, and the model is the only party that can. A run killed over one
malformed patch would throw away the work of every step before it.

`verified-by: bravebot_agent::state::arguments_with_no_patch_are_refused_with_a_sentence_the_model_can_act_on`
`verified-by: bravebot_agent::state::a_refused_patch_is_reported_back_and_the_run_carries_on`

<a id="STATE-8"></a>
### STATE-8: bounding the context relaxes nothing

Actions are the ordinary tools, dispatched through the ordinary path. A write is approved by a
person, a read of a file nobody vouched for is quarantined, a destination is checked against the
same trust map, and the audit trail records the same lines.

**Why it is stated here.** A run whose context is bounded is a run that has forgotten things, and
the tempting mistake is to let a checked state stand in for what was forgotten: to skip an approval
because the state says the user already agreed, or to read a file as trusted because the state says
it was. The state is the model's own note to itself. It is evidence of nothing.

`verified-by: bravebot_agent::state::a_write_in_a_bounded_run_is_still_put_to_the_user`
`verified-by: bravebot_agent::state::a_file_nobody_vouched_for_is_still_quarantined`
`verified-by: bravebot_agent::state::injected_text_in_a_file_cannot_steer_a_bounded_run`
`verified-by: bravebot_agent::state::a_step_is_offered_the_ordinary_tools_and_the_state_patch`

<a id="STATE-9"></a>
### STATE-9: the request is shortened, never the record

The transcript holds every step, and the session record stores it, exactly as with compaction. The
model is the only party working from the state alone: the person watching sees what it saw and what
it has since forgotten.

**Why.** What makes the bound affordable is that nothing is actually lost, only unsent. A person
reviewing a run needs the steps most precisely when the run went wrong, which is the case where the
model's own state is least likely to describe them.

`verified-by: bravebot_agent::state::the_transcript_keeps_what_the_request_dropped`
`verified-by: bravebot_agent::state::the_final_answer_is_recorded_once`

<a id="STATE-10"></a>
### STATE-10: a session may be in this mode, and says which mode it is in

It is a turn loop, so a session may hold it: the user types, the model decides, the user types
again. A manifest run may not, because it fixes every step before the first one and a second prompt
has nothing to join, so asking for an interactive session in that mode is refused rather than
silently downgraded.

A session in a mode that is not the default says so on the opening screen and in `/status`. The
default says nothing, because a line on every session saying `turn` is a line nobody reads.

**Why it is said at all.** Which loop decides the next step changes what the agent can remember, and
that is not something to infer from its behaviour halfway through a task.

`verified-by: bravebot_agent::mode::a_session_may_hold_either_turn_loop_and_not_a_frozen_plan`
`verified-by: bravebot_cli::main::only_a_turn_loop_may_open_an_interactive_session`
`verified-by: bravebot_cli::main::a_bounded_run_can_be_asked_for_on_the_command_line`
`verified-by: bravebot_tui::logo::a_session_in_another_mode_says_so_on_the_opening_screen`
`verified-by: bravebot_tui::logo::an_ordinary_session_does_not_announce_a_mode`
`verified-by: bravebot_tui::logo::a_narrow_pane_still_says_which_mode_it_is_in`
`verified-by: bravebot_tui::status::the_report_names_a_mode_that_is_not_the_default`
`verified-by: bravebot_tui::status::the_report_is_silent_about_the_default_mode`

<a id="STATE-11"></a>
### STATE-11: a step that records nothing is told what it lost

A step that acts and does not update the state is about to forget what it just learned, including
the words it wrote about it, and cannot tell that it has. So it is told, as its next observation.

**Why silence is not neutral here.** This is the mode's defining failure arriving quietly. Asked to
read three files one at a time, a model read the first, said what it held, recorded nothing, and on
the next step had neither the file nor its own sentence about it: it reported the second file's
contents under the first file's name and asked the user for the rest. Nothing was wrong with the
state. There was not one.

A step that did record something is not reminded, or the reminder would be in every request of every
well-behaved run and would stop meaning anything.

`verified-by: bravebot_agent::state::a_step_that_records_nothing_is_told_what_it_lost`
`verified-by: bravebot_agent::state::a_step_that_records_something_is_not_reminded`

<a id="STATE-12"></a>
### STATE-12: the step bound applies even with a person watching

A turn with somebody in front of it carries no round limit, because the person is the better bound.
A bounded run carries one anyway, and reaching it withdraws the tools so the model answers from its
state rather than ending the run.

**Why this differs from a turn.** What makes an unbounded interactive turn safe is that going round
in circles gets slower and dearer every round, since each one re-sends the history, until it is
obvious and meets a context budget. This mode removed exactly that: a request that never grows costs
the same at step five thousand as at step five and looks the same on screen. So the caller's figure
is a ceiling under a floor, and a person can still stop a run at any point.

`verified-by: bravebot_agent::state::an_unbounded_caller_still_gets_the_step_floor`
`verified-by: bravebot_agent::state::a_run_that_spends_its_budget_finishes_with_what_it_has`

<a id="STATE-13"></a>
### STATE-13: a run never ends having said nothing

A reply holding a state update and no words is not an answer. The person cannot see the state, and
it is written in note form for the model's own use, so the run asks for the answer rather than
ending on the note.

**Why.** Observed twice in one session: the work finished, the state recorded it, and the run
stopped, leaving a transcript whose last line was a note to itself. One more request is the
difference between a finished task and a silent one.

`verified-by: bravebot_agent::state::a_run_that_would_end_saying_nothing_is_asked_for_the_answer`

<a id="STATE-14"></a>
### STATE-14: a bounded run is not resumed as one

A session record does not say which mode wrote it, and the state a bounded run decided from is not
written down. So a resumed session is an ordinary turn loop.

**Why.** Resuming into this mode would start from an empty state while the transcript said the work
was underway, which is worse than resuming into a turn: the turn can at least read the record.

`verified-by: by-construction (the resume path passes the default mode, and a session record has no field naming one)`

## Known costs

- **The model can forget something it never wrote down, and cannot tell that it has.** This is the
  mode's defining cost and it is not recoverable at run time: the observation is gone, so nothing
  can notice its absence. What mitigates it is that the instructions say so plainly, and that the
  transcript still holds everything for the person watching.

- **A long-horizon run and a conversation want opposite things.** A person who asks a bounded
  session a follow-up question about what it did two steps ago is asking the one question the mode
  cannot answer. Nothing refuses the question, and the answer will come from the state or not at
  all.
