---
id: MANIFEST
title: Plan then execute
status: normative
governs:
  - crates/core/src/manifest.rs
  - crates/core/src/policy.rs
  - crates/agent/src/manifest.rs
  - crates/agent/src/mode.rs
  - crates/cli/src/main.rs
  - crates/tui/src/sessions.rs
  - crates/tui/src/resume.rs
guards:
  - symbol: Policy::before_planning
  - symbol: Policy::adopt_manifest
  - symbol: Policy::quarantine
  - symbol: Manifest::routing
---

## Scope

One of the other ways a task is run. A turn decides what to do next after each thing it reads. A
manifest run decides everything first: the planner emits a step list, the kernel refuses it or
freezes it, and a driver walks it with no model in the control path.

The default remains the turn loop. This mode is not a stricter policy. The gates are the same;
what changes is the scope of the precommitment, from one turn to a whole run.

A third mode decides step by step as a turn does, from an execution state rather than from the
conversation. It is a different axis from this one and is [skill-state.md](skill-state.md)'s.

## Clauses

<a id="MANIFEST-1"></a>
### MANIFEST-1: planning is gated, not counted

A plan may be asked for only from a planner whose context holds the task string and the driver's
own words, and has been shown nothing else. That is already true everywhere, since untrusted
content is never shown to the planner. The gate is that invariant written where a change has to
pass it.

Two calls happen: the goal in plain words, then the same work fitted to the tool set. What is
forbidden is a re-plan: a plan that fails validation fails the run, and nothing plans again once
a step has read something.

`verified-by: bravebot_core::policy::every_planning_call_is_allowed_from_a_clean_context`
`verified-by: bravebot_core::policy::a_planner_shown_untrusted_content_may_not_plan`
`verified-by: bravebot_agent::manifest::the_audit_trail_records_each_planning_call`
`verified-by: bravebot_agent::manifest::a_manifest_that_is_not_json_fails_without_another_call`

<a id="MANIFEST-2"></a>
### MANIFEST-2: a plan that is not trusted is refused

A plan is a program, so every field in it is a decision. A plan derived from anything an attacker
wrote is an attacker choosing the steps, and there is no repair for that.

`verified-by: bravebot_core::policy::a_plan_from_a_clean_context_is_adopted`
`verified-by: bravebot_core::policy::a_plan_from_a_tainted_context_is_refused`
`verified-by: bravebot_core::policy::a_tainted_planner_is_refused_at_the_call_and_at_adoption`

<a id="MANIFEST-3"></a>
### MANIFEST-3: a run that stopped comes back with what it produced

The plain-words goal, the proposed manifest verbatim, the frozen plan, and what each step did.
On success they travel with the outcome; on failure they travel with the error. A plan that would
not parse has no rendered form, so the model's own words are the only thing left to look at.

Never drop an artefact on the error path, and never make inspecting one conditional on a flag.

`verified-by: bravebot_agent::manifest::a_manifest_that_will_not_parse_comes_back_verbatim`
`verified-by: bravebot_agent::manifest::a_proposal_is_kept_including_its_packaging`
`verified-by: bravebot_agent::manifest::a_plan_that_fails_the_schema_keeps_the_goal_and_the_proposal`
`verified-by: bravebot_agent::manifest::a_step_that_fails_leaves_the_plan_and_everything_that_ran`
`verified-by: bravebot_cli::main::a_failed_plan_is_printed_beside_the_reply`

<a id="MANIFEST-4"></a>
### MANIFEST-4: validation is pure and total

The validator sees a draft and nothing else. Any violation fails the run whole. A manifest is
never half adopted, and a step is never repaired to make a plan usable.

`verified-by: bravebot_core::manifest::an_empty_plan_is_refused`
`verified-by: bravebot_core::manifest::a_tool_outside_the_schema_is_refused`
`verified-by: bravebot_core::manifest::a_path_outside_the_workspace_is_refused`
`verified-by: bravebot_core::manifest::a_read_after_an_action_is_refused`
`verified-by: bravebot_core::manifest::validation_is_deterministic`
`verified-by: bravebot_agent::manifest::a_plan_that_fails_validation_runs_nothing`

<a id="MANIFEST-5"></a>
### MANIFEST-5: the driver adds nothing

It may not insert, skip, reorder, or synthesise a step, and there is no handler for a tool the
schema does not name. If a plan cannot express something, the plan is wrong, not the driver.

`edit_file` is unavailable, because locating a passage means having read the file and the planner
has read nothing. `todo_write` is unavailable because the manifest is already the task list.
There is no shell and no `run`: a command string is destination and payload at once.

`verified-by: bravebot_agent::manifest::injected_text_in_a_file_cannot_add_a_step`
`verified-by: bravebot_agent::manifest::the_planner_is_shown_capabilities_and_no_tool_names`
`verified-by: bravebot_agent::manifest::the_registry_and_the_kernel_schema_describe_the_same_tools`
`verified-by: bravebot_core::manifest::no_action_in_the_schema_fills_a_slot`

<a id="MANIFEST-6"></a>
### MANIFEST-6: routing is locked from the manifest before the first step

Every destination the plan will use is inserted through the gate that refuses anything not
trusted and public, before a single byte has been read. Steps read their destinations out of
that lock, not out of themselves.

Optional routing is filled at validation with an explicit default, so the driver never invents a
destination that did not pass the lock. An omitted listing pattern or search include is locked
as no filter, not as a glob of the empty string, which would match nothing.

`verified-by: bravebot_core::manifest::every_effect_destination_is_named_for_the_routing_lock`
`verified-by: bravebot_core::manifest::an_omitted_search_directory_is_locked_as_the_workspace`
`verified-by: bravebot_core::manifest::an_omitted_list_pattern_is_locked_as_no_filter`
`verified-by: bravebot_agent::manifest::a_write_lands_where_the_plan_said_and_carries_what_it_never_read`
`verified-by: bravebot_agent::manifest::a_listing_with_no_pattern_lists_the_tree`

<a id="MANIFEST-7"></a>
### MANIFEST-7: the only path to a screen is the one the plan named

The slots a person may be shown are named from the manifest before the policy exists. Nothing
widens that set, and nothing releases a slot the plan did not name.

`verified-by: bravebot_core::manifest::only_the_answered_slot_is_named_for_release`
`verified-by: bravebot_agent::manifest::only_the_answered_slot_reaches_the_user`

<a id="MANIFEST-8"></a>
### MANIFEST-8: everything a step produces is quarantined

There is no planner left to show anything to. A step result is stored, whatever its label, and
the only ways out of a slot are a later processor, a write back into the workspace, and a
release the plan named in advance.

An unmarked processor answer is a remark, not a document. Treating it as the slot would write an
explanation over whatever the plan named next.

`verified-by: bravebot_core::policy::a_step_result_is_quarantined_whatever_its_label`
`verified-by: bravebot_agent::manifest::an_unmarked_transform_does_not_become_a_file`
`verified-by: bravebot_agent::manifest::a_slot_no_step_reads_is_never_opened`

<a id="MANIFEST-9"></a>
### MANIFEST-9: the default is the turn loop

An unqualified run is still observe-decide-act, and so is every session: this mode fixes every step
before the first one runs, so a second prompt has nothing to join, and asking for an interactive
session in it is refused rather than quietly downgraded. Manifest is an opt-in on the command line.
An unknown name is refused rather than guessed, because guessing wrong here would silently run the
mode the user did not ask for.

Piped stdin is refused rather than dropped. A pipe is observed context, and this mode does not
observe before it plans. Name a workspace file instead.

`verified-by: bravebot_agent::mode::the_default_is_the_turn_loop`
`verified-by: bravebot_agent::mode::an_unknown_mode_is_refused`
`verified-by: bravebot_cli::main::the_default_mode_is_the_turn_loop`
`verified-by: bravebot_cli::main::an_unknown_mode_is_refused_rather_than_guessed`
`verified-by: bravebot_cli::main::a_leading_mode_flag_is_a_task_not_an_unknown_option`
`verified-by: bravebot_agent::manifest::piped_input_is_refused_rather_than_dropped`
