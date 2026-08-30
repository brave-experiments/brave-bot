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
rather than answered on the user's behalf.

**Why.** The alternative to a person is not a default, it is a guess made in their name. The
planner is told a reply came from a person, so inventing one would be worse than not asking.

`verified-by: bravebot_agent::turn::an_unattended_run_declines_every_question_in_the_series`
`verified-by: bravebot_agent::turn::a_refused_write_does_not_happen`

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

It prints the endpoint, whether a premium host is configured, the key id, the model in force and
whether it was chosen or defaulted, the confinement available on this platform, and the state of
any imported subscription. The signing key is named as never transmitted. A configuration error
makes it fail rather than pass with a warning.

**Why.** It exists to answer "what will this actually use", so reporting a default when a choice
is in force would explain the wrong thing.

`verified-by: none`
