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

`--dangerously-skip-permissions` is the one way a run nobody is watching may write, and it is the
person's own instruction rather than a default: what it selects, and what it costs, is
[permission-modes.md](permission-modes.md). The planner's questions are declined in that mode too,
because they are not permissions.

**Why.** The alternative to a person is not a default, it is a guess made in their name. The
planner is told a reply came from a person, so inventing one would be worse than not asking. A flag
somebody typed is not a guess, which is what makes it the only thing that may lift the first half of
this and nothing that may lift the second.

`verified-by: bravebot_agent::turn::an_unattended_run_declines_every_question_in_the_series`
`verified-by: bravebot_agent::turn::a_refused_write_does_not_happen`
`verified-by: bravebot_cli::main::permissions_are_enforced_unless_the_flag_is_given`

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

It prints every backend this build can reach and what identifies it, which names the settings set,
which settings files are in force and which of them won a name more than one set, the model in force
and whether it was chosen or defaulted, the confinement available on this platform, and the state of
any imported subscription. The signing key is named as never transmitted, and a value from a settings
file is never printed: where a credential decides whether a backend works, what is reported is that
one was found. A configuration error makes it fail rather than pass with a warning.

**Why.** It exists to answer "what will this actually use", so reporting a default when a choice
is in force would explain the wrong thing, and naming one backend where two are reachable would
explain only the half somebody happened to ask about. Naming the files is the same argument: settings
resolve across three of them, so a value somebody did not expect has three places it could have come
from and the path is the whole of what narrows it to one. Values are withheld because a settings file
holds credentials on some machines, and a diagnostic that prints one is a diagnostic people paste
into issues. Whether one was found still has to be said, because a backend nothing can authenticate
is the case this is most often run to explain.

`verified-by: bravebot_cli::main::a_gateway_credential_is_reported_as_found_and_never_printed`
`verified-by: bravebot_cli::main::a_gateway_with_no_credential_is_reported_as_having_none`

<a id="CLI-8"></a>
### CLI-8: `--mode` chooses how a one-shot is run; the default is the turn loop

`turn` observes and decides step by step, which is what an unqualified `bravebot "task"` has
always been. `manifest` plans the whole run first, then executes it. An unknown name is refused
rather than guessed. Both modes are unattended, with an empty trust map: where nobody can be
asked, nothing is approved unless the flag in CLI-1 says otherwise.

This is a different axis from the mode in [permission-modes.md](permission-modes.md), and the two
compose. `--mode` decides when control flow is settled; the other decides who answers a prompt.

A failed plan is printed on stderr even without `--trace`, because otherwise a one-line complaint
is all that remains of a document nobody can see. The plan never shares stdout with the reply.

`verified-by: bravebot_cli::main::the_default_mode_is_the_turn_loop`
`verified-by: bravebot_cli::main::a_leading_mode_flag_is_a_task_not_an_unknown_option`
`verified-by: bravebot_cli::main::an_unknown_mode_is_refused_rather_than_guessed`
`verified-by: bravebot_cli::main::a_failed_plan_is_printed_beside_the_reply`
`verified-by: bravebot_agent::manifest::an_unattended_manifest_run_does_not_write`

<a id="CLI-9"></a>
### CLI-9: a one-shot run names its own model, or asks for the one a session would

`--model <name>` names the model for one run and outranks everything else. Where no flag names
one, the model is the one a session opening in the same directory would ask for: the choice
`/model` recorded, then the configured model, which is an exported
`BRAVE_AI_CHAT_DEFAULT_MODEL`, then the settings file's `model` key, then the default the build was
made with. A name is resolved against the configuration wherever it was written: `opus`, `sonnet`
and `haiku` name the tier's own model and the older spelling of the routing entry names the current
one, so the flag and the settings key it outranks accept the same spellings. A `--model` with no
name after it, or a blank one, is refused rather than read as no choice.

**Why.** A script that cannot name a model has only one route to a particular one, which is for
somebody to open the interface and pick it, and in a pipeline that is not a route at all. The flag
is that route, and it ranks above every other because it names a model for one invocation and
nothing else: two scripts in the same checkout can ask for different models, which nothing a file
records can do.

Below the flag, a run resolves a model the way a session does, so the two surfaces reach the same
models by the same names and a script needs no interactive step to use the one somebody already
chose. Ranking configuration above the record instead would mean a person who picked a model could
not run a script with it, and a script wanting a different one from the picked one has the flag.

Resolving against the configuration rather than at parse, because a tier word names a model only
the configuration knows: the AWS account's own model for that tier where it named one, and Brave's
name for it otherwise. A flag that sent such a word as written would refuse a spelling the file it
overrides takes, and be answered by whatever the service substitutes for a name it has never heard
of.

A blank name is refused because a script that computed an empty variable asked for a model.
Reading the blank as no choice would answer it with whatever was recorded or configured and say
nothing about having done so, which is the substitution the flag exists to make impossible.

`verified-by: bravebot_cli::main::a_model_flag_names_the_model_a_run_asks_for`
`verified-by: bravebot_cli::main::a_run_that_named_no_model_names_nothing`
`verified-by: bravebot_cli::main::the_command_line_outranks_the_record_a_session_would_read`
`verified-by: bravebot_cli::main::a_run_that_named_no_model_reads_the_record_a_session_would`
`verified-by: bravebot_cli::main::a_run_with_nothing_to_go_on_leaves_the_configured_model_in_force`
`verified-by: bravebot_cli::main::a_model_name_is_carried_as_it_was_typed`
`verified-by: bravebot_cli::main::a_tier_word_on_the_command_line_names_the_model_the_settings_key_would`
`verified-by: bravebot_config::lib::a_name_from_anywhere_resolves_as_the_settings_key_does`
`verified-by: bravebot_cli::main::a_model_flag_with_no_name_is_refused`
`verified-by: bravebot_cli::main::a_blank_model_is_refused_rather_than_read_as_no_choice`

<a id="CLI-10"></a>
### CLI-10: a model a run named, and did not get, fails the run

Where `--model` named a model and the endpoint answered with a different one, both names are said
on stderr and the run exits non-zero. The reply still goes to stdout, and stdout carries nothing
else. Three cases are not this: a run that named no model, a name that asks for whichever model the
server picks rather than for a particular one, and a backend that does not report the name it was
asked for.

**Why.** A model a run cannot be served is substituted rather than refused. One that needs a
subscription is answered by whatever the free tier serves, with an ordinary reply and nothing to
distinguish it, so the name the server reports is the only trace there is. A person watching a
session is shown that line and can act on it; a script sees a reply that looks exactly like the one
it wanted, and a model is pinned in the first place because of what a run costs or how good its
output has to be. The status is the part of a finished run a script is certain to read, which is
what makes it the thing that has to carry this.

The three exclusions are the cases where a different name is not a substitution. A run that named
no model asked for nothing in particular. A routing entry resolves to a model per request, which is
what it is for. A backend asked by an opaque handle answers with a name that never matched what
went in, so comparing them would fail every run made against one.

`verified-by: bravebot_cli::main::a_model_a_run_named_and_did_not_get_is_reported`
`verified-by: bravebot_cli::main::a_model_that_answered_as_asked_is_no_complaint`
`verified-by: bravebot_cli::main::a_run_that_named_no_model_is_not_failed_by_the_one_that_answered`
`verified-by: bravebot_cli::main::a_routing_entry_answered_by_a_model_is_not_a_substitution`
`verified-by: bravebot_cli::main::a_backend_that_does_not_report_what_it_was_asked_is_not_compared`
`verified-by: bravebot_cli::main::a_substituted_model_is_reported_beside_the_reply_never_in_it`
`verified-by: bravebot_cli::main::a_run_answered_by_another_model_does_not_succeed`

<a id="CLI-11"></a>
### CLI-11: `--add-dir` makes a directory reachable, and vouches for nothing

`--add-dir <path>` opens a directory outside the working one for the length of the run, and may be
given more than once. An absolute path that exists, is a directory, and is not already inside the
working one is opened; anything else is refused by name and the run stops before the turn. The
run's trust map stays empty, so a file read there is read on the same footing as the project's own
files: nothing vouched for it. A write there is refused as any other write in an unattended run is,
and the flag in CLI-1 lifts that exactly as it does elsewhere.

**Why.** A headless task pointed at one checkout often needs to read another, and an absolute path
outside the working directory is otherwise refused whatever else is true, so without this the task
cannot be done at all.

Vouching is a separate grant, and it is the one an unattended run cannot make. The interactive
command of the same name records that a person vouched for the directory, which it can do because a
person typed it in a session whose map already holds their answer about the directory they are
working in. A run nobody is watching holds no such answer, its own working directory included, so a
rule trusting the tree named on the command line would leave it more trusted than the tree the run
works in. Reaching a directory is what the work needs; trusting what is in it is not.

Stopping rather than carrying on, because the two audiences differ: a session says the path was not
opened and leaves the person to retype it, and a script that carried on would fail somewhere further
in, over a file it was told it could open.

`verified-by: bravebot_cli::main::a_directory_flag_names_a_directory_the_run_may_reach`
`verified-by: bravebot_cli::main::the_directory_flag_is_repeatable`
`verified-by: bravebot_cli::main::a_directory_flag_with_no_path_is_refused`
`verified-by: bravebot_cli::main::a_directory_the_command_line_named_is_reachable`
`verified-by: bravebot_cli::main::a_directory_that_cannot_be_opened_stops_the_run`
`verified-by: bravebot_core::trust::an_empty_store_trusts_nothing`
