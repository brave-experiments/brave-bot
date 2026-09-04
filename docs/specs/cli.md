---
id: CLI
title: The command line
status: normative
governs:
  - crates/cli/src/main.rs
---

## Scope

Running bravebot without the interactive interface: a one-shot task, piped input, `doctor`, and
what goes where on the way out. The interactive session is
[terminal-input.md](terminal-input.md) and [terminal-transcript.md](terminal-transcript.md).

A one-shot run has nobody to ask, and most of what makes it different follows from that.

## Clauses

<a id="CLI-1"></a>
### CLI-1: where nobody can be asked, nothing is approved

Effects are refused rather than applied unseen, and the planner's own questions are declined
rather than answered on the user's behalf. Every write is put to the confirmer here, including the
ones an interactive session does quietly, and refused there.

**Why.** The alternative to a person is not a default, it is a guess made in their name. The
planner is told a reply came from a person, so inventing one would be worse than not asking. A
write of the turn's own trusted output is done quietly while somebody is following the session,
and quietly is not the same as unseen: there is nobody following a cron job.

`verified-by: bravebot_agent::turn::an_unattended_run_declines_every_question_in_the_series`
`verified-by: bravebot_agent::turn::a_refused_write_does_not_happen`
`verified-by: bravebot_agent::manifest::an_unattended_manifest_run_does_not_write`

<a id="CLI-2"></a>
### CLI-2: stdin is read only when it is not a terminal

A terminal's stdin is left alone, so an interactive invocation does not sit waiting for input
nobody is sending. Piped bytes are read when there are any.

`verified-by: bravebot_cli::main::a_terminal_stdin_is_not_read`
`verified-by: bravebot_cli::main::piped_bytes_are_read_when_stdin_is_not_a_terminal`

<a id="CLI-3"></a>
### CLI-3: piped input is untrusted and private, always

Nothing vouched for what a pipe carries: `gh pr diff` and `cat build-error.txt` both arrive the
same way and neither passed through the trust map. So it is quarantined and the planner is given a
reference, never the bytes.

**Why.** A pipe has no path, so there is nothing for the trust map to have an opinion about. The
pessimistic label is the only one that holds without knowing what fed it.

`verified-by: bravebot_core::policy::piped_input_is_labelled_untrusted_and_private`
`verified-by: bravebot_core::policy::piped_input_is_quarantined_when_presented`
`verified-by: bravebot_agent::turn::piped_input_is_never_shown_to_the_planner`

<a id="CLI-4"></a>
### CLI-4: input over the cap is refused, and says what to do instead

Rather than truncated, since a silently shortened input is one the planner would answer about
having seen part of.

`verified-by: bravebot_cli::main::input_over_the_cap_is_refused`

<a id="CLI-5"></a>
### CLI-5: stdout carries the reply and nothing else

Progress, errors and the audit trail go to stderr, so a one-shot run is pipeable. `--trace` puts
the trail on stderr beside it: which gate checked what, the label every value carried, and what
was released.

**Why.** A progress line mixed into stdout would corrupt whatever the user piped the reply into.

`verified-by: bravebot_cli::main::stdout_carries_the_reply_and_nothing_else`
`verified-by: bravebot_cli::main::an_untraced_run_writes_no_trail`
`verified-by: bravebot_cli::main::the_trail_renders_a_line_for_every_event`

<a id="CLI-6"></a>
### CLI-6: a failure exits non-zero

A configuration error, a refused argument, and a turn that could not run all fail rather than
exiting successfully with an explanation on stdout.

`verified-by: none`

<a id="CLI-7"></a>
### CLI-7: `doctor` reports configuration and confinement without changing anything

It prints every backend this build can reach and what identifies it, which names a settings file
set, the model in force and whether it was chosen or defaulted, the confinement available on this
platform, and the state of any imported subscription. The signing key is named as never
transmitted, and a value from the settings file is never printed. A configuration error makes it
fail rather than pass with a warning.

**Why.** It exists to answer "what will this actually use", so reporting a default when a choice
is in force would explain the wrong thing, and naming one backend where two are reachable would
explain only the half somebody happened to ask about. Values are withheld because a settings file
holds credentials on some machines, and a diagnostic that prints one is a diagnostic people paste
into issues.

`verified-by: none`

<a id="CLI-8"></a>
### CLI-8: `--mode` chooses how a one-shot is run; the default is the turn loop

`turn` observes and decides step by step, which is what an unqualified `bravebot "task"` has
always been. `manifest` plans the whole run first, then executes it. An unknown name is refused
rather than guessed. Both modes are unattended, with an empty trust map: where nobody can be
asked, nothing is approved.

A failed plan is printed on stderr even without `--trace`, because otherwise a one-line complaint
is all that remains of a document nobody can see. The plan never shares stdout with the reply.

`verified-by: bravebot_cli::main::the_default_mode_is_the_turn_loop`
`verified-by: bravebot_cli::main::a_leading_mode_flag_is_a_task_not_an_unknown_option`
`verified-by: bravebot_cli::main::an_unknown_mode_is_refused_rather_than_guessed`
`verified-by: bravebot_cli::main::a_failed_plan_is_printed_beside_the_reply`
`verified-by: bravebot_agent::manifest::an_unattended_manifest_run_does_not_write`

<a id="CLI-9"></a>
### CLI-9: `--disable-vetting` quarantines rather than offering anything to a checker

A file nobody vouched for is normally offered to an isolated checker before a turn is refused its
contents. This turns that off, and then such a file is simply quarantined and the turn works
through a reference as it would have anyway.

**Why.** A checker is a model call, so it costs a request and reaches the network. A run that must
make no call it was not asked to make needs a way to say so, and telling whether a verdict was what
made a difference needs the same switch.

`verified-by: bravebot_agent::turn::vetting_can_be_turned_off_and_then_nothing_is_offered_to_a_checker`
