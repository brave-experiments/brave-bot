---
id: SHELL
title: Shell mode
status: normative
governs:
  - crates/agent/src/shell.rs
  - crates/tui/src/app.rs
guards:
  - symbol: Policy::label_user_command_output
---

## Scope

The `!` prompt: a line the user typed, run through a real shell. It is not a tool and the planner
cannot reach it. The planner's own path for running a program is
[tools/run.md](tools/run.md), and is a different thing entirely.

## Why this is not a hole in the shell exclusion

Read the exclusion precisely. It is not "shell strings are dangerous bytes". It is that a person
cannot endorse a routing field a shell string does not have, and the reason one must be endorsed
is that the string came from the **planner**, which an attacker may have steered. Provenance is
what the rule is about, so a line the user typed is outside it.

## Clauses

### SHELL-1: `!` on an empty prompt runs the line through `$SHELL -c`

Globs, `$VAR`, redirection, `&&` and `$(...)` all work as they do in the user's terminal. The
marker is a mode rather than a character: the prompt changes colour, backspace or escape leaves,
and the mode lasts one command. `$SHELL` falls back to a POSIX shell when unset. An empty line is
not run.

`verified-by: bravebot_agent::shell::a_glob_is_expanded_by_the_shell`
`verified-by: bravebot_agent::shell::a_variable_is_expanded_by_the_shell`
`verified-by: bravebot_agent::shell::a_redirection_writes_the_file`
`verified-by: bravebot_agent::shell::stages_are_piped_together`
`verified-by: bravebot_agent::shell::commands_joined_with_and_both_run`
`verified-by: bravebot_agent::shell::the_shell_falls_back_to_a_posix_one_when_the_variable_is_unset`
`verified-by: bravebot_agent::shell::an_empty_line_is_not_run`
`verified-by: bravebot_tui::shell_mode::the_prompt_marker_changes_in_shell_mode`
`verified-by: bravebot_tui::shell_mode::the_hint_names_the_shell_while_in_shell_mode`

### SHELL-2: nothing asks

The approval prompt exists so a person endorses argv the **planner** proposed. Here the person is
the one it would have asked, so confirming their own keystroke would be theatre. `! rm -rf build`
simply runs.

`verified-by: bravebot_agent::shell::the_command_is_recorded_as_something_the_user_did`

### SHELL-3: the output is `(T,priv)` and reaches the planner in full

Not as a reference. This is the difference from a program the planner ran itself: after
`! cargo test` the user can say "fix the first failure" and the planner has already read the
errors. Output from a failing command reaches it too, since that is where the explanation is. A
cancelled command records nothing.

This is a first label from provenance, exactly like the label on a program's output or on the
user's own configuration. It is admissible for the reason a vouched-for command is: a person took
responsibility, and nothing inspected anything.

`verified-by: bravebot_agent::shell::what_a_command_printed_reaches_the_planners_context`
`verified-by: bravebot_agent::shell::a_failing_commands_output_still_reaches_the_planner`
`verified-by: bravebot_agent::shell::trusting_the_output_is_recorded_in_the_trail`
`verified-by: bravebot_agent::shell::a_cancelled_command_records_nothing`
`verified-by: bravebot_agent::shell::what_is_shown_is_what_the_gate_released`
`verified-by: bravebot_tui::shell_mode::output_is_not_drawn_as_quarantined`
`verified-by: bravebot_tui::shell_mode::output_cannot_draw_its_own_escapes`

### SHELL-4: only a line a human typed

Never argv the planner proposed, never text read from a file, never anything a processor produced,
never a line reconstructed from a transcript.

**Why.** The justification cannot be checked from the bytes, so it lives at the call site. Today
that is the TUI's shell mode and nothing else.

`verified-by: none`

### SHELL-5: the planner gets no shell tool, ever

Not behind a capability, not behind an approval prompt, not via MCP. If it could ask for one,
everything above is void.

`verified-by: none`

## Known costs

- **`! cat notes-from-a-stranger.md` puts somebody else's words into the planner's context as
  though they were the user's.** Nothing inspects the bytes to catch that, exactly as nothing
  inspects a directory that was vouched for. It is the same assertion a person makes by vouching for a
  command at a run prompt, made once for one command. If the user would not press `a` for it, they should ask the agent to `run` it
  instead and have the output quarantined.
