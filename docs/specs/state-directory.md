---
id: STATE
title: The state directory
status: normative
governs:
  - crates/agent/src/home.rs
  - crates/tui/src/store.rs
  - crates/tui/src/update.rs
  - crates/skus/src/store.rs
guards:
  - symbol: home::create_directory
  - symbol: home::write_file
  - symbol: home::append_to_file
---

## Scope

`~/.bravebot`, the directory holding what outlives a session, and who on the machine may read what
is written into it. The prompt history, the model, theme, effort and editing choices, the answer to
the update question, session records, skills, standing instructions and an imported subscription
all live here.

What each of those files means belongs to the spec for that subject:
[sessions.md](sessions.md) for a session record, [skills.md](skills.md) and
[instructions.md](instructions.md) for what is read out of the directory,
[premium-credentials.md](premium-credentials.md) for the subscription, and
[incognito.md](incognito.md) for the mode that writes none of it. This file covers the directory
itself.

## Clauses

<a id="STATE-1"></a>
### STATE-1: the state directory and everything written into it is readable only by the user

On Unix, a directory under `~/.bravebot` is created with mode 0700 and a file written into one
with mode 0600, whichever subsystem is doing the writing. A file's mode is asked for as it is
created rather than applied once it holds anything, and a directory or file that is already there
is narrowed rather than left as it was found. Narrowing walks up from what was written as far as
the state directory and stops there.

**Why.** The history file is every prompt anybody has typed into this program: the paths they were
working on, the branch names, and whatever they pasted into one. At the process umask that is
world-readable, which on a shared build host or a multi-user machine means readable by every local
account. The other files say less on their own but sit in the same directory and are written by the
same code, and a rule covering some of the files in a directory is one nobody could hold a diff
against.

Narrowing what is already there rather than only what is created new is what makes this reach a
machine that has run an older build, which is every machine that has the history worth protecting.
Creating a directory that exists succeeds without touching its mode, so a fix that only set modes
on creation would leave exactly those machines as they were.

Stopping at the state directory bounds it in the other direction. Whose home this is, and what else
is kept in it, is the user's own business, and a program that narrowed directories it was never
asked about would be making decisions outside anything it was given.

One helper does this for the crates that can share one. The crate that imports a subscription
depends on nothing, as [layering.md](layering.md) records, and keeps its own copy of the modes
rather than taking a dependency for four lines. It creates the state directory when it is the
first to write, which is the case that made the mode of a directory holding prompt history a
matter of which subsystem ran first.

`verified-by: bravebot_agent::home::a_directory_is_created_reachable_only_by_its_owner`
`verified-by: bravebot_agent::home::a_directory_left_open_by_an_older_build_is_narrowed`
`verified-by: bravebot_agent::home::narrowing_stops_at_the_state_directory`
`verified-by: bravebot_agent::home::a_file_is_written_readable_only_by_its_owner`
`verified-by: bravebot_agent::home::a_file_left_readable_by_an_older_build_is_narrowed`
`verified-by: bravebot_agent::home::an_appended_file_is_readable_only_by_its_owner`
`verified-by: bravebot_tui::state_directory::the_prompt_history_is_readable_only_by_its_owner`
`verified-by: bravebot_tui::state_directory::a_rewritten_history_is_readable_only_by_its_owner`
`verified-by: bravebot_tui::state_directory::a_recorded_choice_is_readable_only_by_its_owner`
`verified-by: bravebot_tui::state_directory::a_state_directory_an_older_build_left_open_is_narrowed`
`verified-by: bravebot_tui::state_directory::nothing_above_the_state_directory_is_touched`
`verified-by: bravebot_tui::state_directory::writing_a_session_narrows_the_state_directory`
`verified-by: bravebot_skus::store::the_directory_it_is_kept_in_is_not_reachable_by_anyone_else`

<a id="STATE-2"></a>
### STATE-2: one definition of where the directory is, and no fallback

Every subsystem resolves `~/.bravebot` through the same answer, and a machine with no `HOME` has no
state directory rather than a guessed one. Nothing is read and nothing is written in that case, and
each caller does without.

**Why.** Two definitions of where the directory is would eventually disagree, and the disagreement
would show up as a choice that does not stick or a history that is written twice. Inventing a
location where `HOME` says nothing is worse than doing without: it would mean reading files from
somewhere the user never chose, and this is the one directory whose contents are trusted for being
the user's own.

`verified-by: bravebot_agent::home::the_home_directory_is_the_one_the_environment_names`
`verified-by: bravebot_agent::home::an_absent_home_is_not_an_error`
`verified-by: bravebot_agent::home::an_empty_home_is_treated_as_no_home_at_all`
`verified-by: bravebot_skus::store::no_home_directory_is_reported_rather_than_guessed`
