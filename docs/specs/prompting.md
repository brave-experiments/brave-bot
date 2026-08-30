---
id: PROMPT
title: Asking a person
status: normative
governs:
  - crates/tui/src/confirm.rs
  - crates/tui/src/trust_prompt.rs
  - crates/tui/src/remote_confirm.rs
---

## Scope

Every moment the system stops and puts something to a human: what the prompt must show, what an
answer grants, and what one answer must never be taken for. `ask_user`, where the **planner** asks
a question, is [tools/ask-user.md](tools/ask-user.md) and is a different thing: these prompts are the system asking
permission.

There are five: the startup trust question, a read of a file nobody vouched for, a write or edit, a
run, and reading what a run printed.

## What every prompt owes the reader

### PROMPT-1: a prompt shows what is actually at stake, not a summary of it

A write prompt shows the path and the body; an overwrite shows what it replaces; a run prompt
shows the argv, the resolved binary and the directory; an output prompt shows the bytes and the
command that printed them. A person cannot endorse a routing field they were not shown.

`verified-by: bravebot_tui::confirm::a_new_file_prompt_shows_the_path_and_body`
`verified-by: bravebot_tui::confirm::an_overwrite_prompt_shows_what_it_replaces`
`verified-by: bravebot_tui::confirm::a_run_prompt_shows_the_argv_the_binary_and_the_directory`
`verified-by: bravebot_tui::confirm::the_output_prompt_shows_the_bytes_and_the_command`

### PROMPT-2: a prompt says what approving does, and what it does not

The run prompt says it is not sandboxed, asks for the side effects and the output together, and
names the exact command it would vouch for. The output prompt says
what approving does. The trust prompt explains the consequence and names both answers.

**Why.** The second half of a run grant, that what the command prints becomes trusted, is the one
nothing else would tell the user.

`verified-by: bravebot_tui::confirm::a_run_prompt_says_it_is_not_sandboxed`
`verified-by: bravebot_tui::confirm::a_run_prompt_asks_for_the_side_effects_and_the_output_together`
`verified-by: bravebot_tui::confirm::a_run_prompt_names_the_exact_command_it_would_vouch_for`
`verified-by: bravebot_tui::confirm::the_output_prompt_says_what_approving_does`
`verified-by: bravebot_tui::trust_prompt::the_prompt_explains_the_consequence`
`verified-by: bravebot_tui::trust_prompt::the_prompt_names_the_directory_and_both_answers`

### PROMPT-3: what a prompt shows is drawn inside a margin it cannot forge

Content in a prompt is untrusted like any other. An untrusted body is marked as such,
and command output is drawn inside the margin.

`verified-by: bravebot_tui::confirm::an_untrusted_body_is_marked_in_the_prompt`
`verified-by: bravebot_tui::confirm::output_is_drawn_inside_the_margin_it_cannot_forge`

### PROMPT-4: a review stays legible, or says it could not

A long body keeps the question on screen and offers the rest, which can be scrolled to. A small
edit in a large file shows only the change. An empty output says so. A diff that cannot be
computed says so rather than showing nothing.

**Why.** Reviewing a whole file body on a terminal is not review, which is why `edit_file` exists
on a passage rather than a whole body. A prompt that scrolled the question away would be collecting a keypress, not a decision.

`verified-by: bravebot_tui::confirm::a_long_body_keeps_the_question_on_screen_and_offers_the_rest`
`verified-by: bravebot_tui::confirm::the_rest_of_a_long_body_can_be_scrolled_to`
`verified-by: bravebot_tui::confirm::a_small_edit_in_a_large_file_shows_only_the_change`
`verified-by: bravebot_tui::confirm::output_that_is_empty_says_so`
`verified-by: bravebot_tui::confirm::an_uncomputable_diff_says_so`

## What an answer means

### PROMPT-5: one answer is never taken for another

An approved write does not approve a run. A write approval is not an answer to a question, and an
answer to a question is not consent to a write. Each endorsement is single-use and bound to the
exact value it was given for.

**Why.** These are separate grants that happen to use the same keyboard.

`verified-by: bravebot_tui::remote_confirm::an_approved_write_does_not_approve_a_run`
`verified-by: bravebot_tui::remote_confirm::a_write_approval_is_not_taken_as_an_answer_to_a_question`
`verified-by: bravebot_tui::remote_confirm::an_answer_to_a_question_is_not_taken_as_consent_to_a_write`

### PROMPT-6: standing permission needs its own key, and is never the default

The run prompt separates running once from running always, Enter does not approve a run, and a run
releasing private data offers no standing permission at all. Declining, and Ctrl-C, vouch
for nothing.

`verified-by: bravebot_tui::confirm::the_run_keys_separate_running_once_from_running_always`
`verified-by: bravebot_tui::confirm::enter_does_not_approve_a_run`
`verified-by: bravebot_tui::confirm::a_run_that_releases_private_data_offers_no_standing_permission`
`verified-by: bravebot_tui::confirm::saying_no_to_a_run_vouches_for_nothing`
`verified-by: bravebot_tui::confirm::ctrl_c_refuses_the_run_and_vouches_for_nothing`
`verified-by: bravebot_tui::trust_prompt::declining_trusts_nothing`

### PROMPT-7: declining is not cancelling

Saying no to a write does not stop the turn; Ctrl-C refuses it and does. Leaving at the startup
question ends the session, and only Ctrl-C leaves.

**Why.** A refusal the agent can carry on past is how a person steers without starting over.

`verified-by: bravebot_tui::confirm::saying_no_does_not_stop_the_turn`
`verified-by: bravebot_tui::confirm::ctrl_c_refuses_the_write_and_stops_the_turn`
`verified-by: bravebot_tui::trust_prompt::ctrl_c_leaves_rather_than_answering_the_question`
`verified-by: bravebot_tui::trust_prompt::only_ctrl_c_leaves`
`verified-by: bravebot_tui::trust_prompt::leaving_starts_no_session`

### PROMPT-8: a resume restores standing permissions, and nothing else

Two of these grants are standing: the trust map, and the list of commands a person said to stop
asking about. Both are written into the session record and come back with `--resume`, because the
person resuming is the person who gave them. A fresh session in the same directory restores
neither and asks again.

Nothing else survives. A single-use endorsement is created by one approval, is bound to one value,
and is never written down, so a resumed turn cannot replay a write or a run that an earlier turn
was allowed. Answers to the planner's own questions are remembered only in the live session, so a
resumed session puts them again.

**Why.** A standing permission is a decision about the future that its owner made deliberately. An
endorsement is a decision about one act that has already happened, and reviving one would be
approving something nobody looked at.

`verified-by: bravebot_core::policy::a_turn_inherits_what_the_session_vouched_for`
`verified-by: bravebot_core::policy::an_endorsement_cannot_be_replayed`
`verified-by: bravebot_tui::sessions::sessions_are_written_read_back_and_kept_per_directory`

### PROMPT-9: where nobody can be asked, the answer is no

A one-shot run refuses effects rather than applying them unseen, and declines every question
rather than inventing an answer. A closed channel refuses a run and answers no
question.

`verified-by: bravebot_tui::remote_confirm::a_closed_channel_refuses_a_run`
`verified-by: bravebot_tui::remote_confirm::a_closed_channel_answers_no_question`
`verified-by: bravebot_tui::remote_confirm::a_dropped_answer_channel_answers_no_question`
`verified-by: bravebot_tui::remote_confirm::a_refusal_travels_back_too`
`verified-by: bravebot_agent::turn::an_unattended_run_declines_every_question_in_the_series`
