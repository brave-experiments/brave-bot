---
id: CMD
title: Slash commands
status: normative
governs:
  - crates/tui/src/app.rs
guards:
  - symbol: COMMANDS
---

## Scope

A line beginning with `/` that this program acts on itself, in place of sending it anywhere. What
every one of them shares: where such a line may come from, when a line is one, and what happens to
the line once it is.

Not what any particular command then does. `/add-dir` and `/status` are the trust map's, in
[trust-map.md](trust-map.md); `/compact` is [compaction.md](compaction.md)'s; `/clear` begins a
session, which is [sessions.md](sessions.md)'s. The `!` prompt is a different surface entirely and
is [shell-mode.md](shell-mode.md).

**Skills are not on this surface.** Other agents let a person type a skill's name after a slash,
and this one does not: a skill is advertised to the planner by name and description, and its body
is fetched by the planner asking for it. Nothing in the input box knows skills exist, so
`/commit-style` is a prompt like any other sentence. [skills.md](skills.md) owns what a skill is
and what each source is trusted for, and [tools/load-skill.md](tools/load-skill.md) owns the
fetch. CMD-7 says why the two surfaces stay apart.

## Where a command may come from

<a id="CMD-1"></a>
### CMD-1: only a line a person typed into the box

A command is dispatched from a key press in the input box and from nowhere else. Never a line the
planner produced, never text read out of a file, never anything a processor returned, never a line
reconstructed from a transcript. A model that writes `/clear` has written four characters, and
they reach a person's screen as four characters.

**Why.** Every command here decides something a turn is not allowed to decide on its own: which
directories are reachable, what the conversation consists of, which model thinks. The endorsement
is the keystroke, so the keystroke is the only thing that may produce one. Recalling an earlier
prompt is still a person's own line, so a recalled `/status` is a command again.

`verified-by: by-construction (dispatch is a branch of the input box's key handler, reached only from a key press, and no path carries model output, file content or processor output into it)`


<a id="CMD-2"></a>
### CMD-2: the whole word, and an argument only after a space

`/rename` is the command. `/renamed the parser` is a prompt, because the word is longer.
`what does /add-dir do` is a prompt, because the word is not the line. The bare word with nothing
after it is the command with an empty argument, answered by saying what it needs rather than by
doing nothing quietly.

**Why.** The set of words this program claims is taken out of the language a person can use to
talk to the planner, so it is claimed as narrowly as possible: asking how a command works must
stay a question. Prefix matching would have made `/add-dirs are useful` open a directory called
`s are useful`.

`verified-by: bravebot_tui::app::a_longer_word_starting_with_the_command_is_a_prompt`
`verified-by: bravebot_tui::app::an_argument_is_taken_only_after_the_whole_command_word`
`verified-by: bravebot_tui::app::a_prompt_containing_the_add_dir_command_is_still_a_prompt`
`verified-by: bravebot_tui::app::a_prompt_containing_the_status_command_is_still_a_prompt`
`verified-by: bravebot_tui::app::a_prompt_containing_the_clear_command_is_still_a_prompt`
`verified-by: bravebot_tui::app::a_prompt_containing_the_compact_command_is_still_a_prompt`
`verified-by: bravebot_tui::app::a_prompt_containing_the_model_command_is_still_a_prompt`
`verified-by: bravebot_tui::app::a_prompt_containing_the_theme_command_is_still_a_prompt`
`verified-by: bravebot_tui::app::a_longer_word_starting_with_theme_is_a_prompt`
`verified-by: bravebot_tui::app::a_prompt_containing_the_rename_command_is_still_a_prompt`
`verified-by: bravebot_tui::app::a_prompt_containing_the_exit_command_is_still_a_prompt`
`verified-by: bravebot_tui::app::the_bare_add_dir_command_is_still_the_command`
`verified-by: bravebot_tui::app::the_bare_rename_command_is_still_the_command`
`verified-by: bravebot_tui::app::a_prompt_containing_the_loop_command_is_still_a_prompt`
`verified-by: bravebot_tui::app::a_longer_word_starting_with_loop_is_a_prompt`
`verified-by: bravebot_tui::app::the_bare_loop_command_is_still_the_command`


<a id="CMD-3"></a>
### CMD-3: in shell mode the line is a command line, not a command

With the `!` mode armed, `/status` is a program somebody may have and is run as one. Nothing is
offered for completion there either, since `/usr/bin/env` is a path.

**Why.** The mode is how a person says which of the two they meant, and it is the more specific
statement of the two. Completing in it would rewrite the line under somebody typing a path.

`verified-by: bravebot_tui::app::a_slash_command_in_shell_mode_is_a_command_line`
`verified-by: bravebot_tui::app::shell_mode_offers_no_completions`

## What a command does to the line

<a id="CMD-4"></a>
### CMD-4: a command is never sent as a prompt

The line comes off the box and nothing enters the conversation. A session asked to shorten itself
must not answer by talking about shortening itself, and a session asked to clear must not answer
by asking what to clear.

That a command may go on to start a request of its own is a separate thing: `/compact` sends a
conversation to be summarised, which [compaction.md](compaction.md) governs, and `/model` reaches
the network to list models. Neither sends the typed line.

`verified-by: bravebot_tui::app::typing_the_status_command_reports_rather_than_prompting`
`verified-by: bravebot_tui::app::typing_the_clear_command_starts_a_new_session`
`verified-by: bravebot_tui::app::the_compact_command_asks_for_a_summary_rather_than_being_sent`
`verified-by: bravebot_tui::app::the_add_dir_command_carries_its_directory`
`verified-by: bravebot_tui::app::typing_the_model_command_opens_the_picker`
`verified-by: bravebot_tui::app::typing_the_theme_command_opens_the_picker`
`verified-by: bravebot_tui::app::the_theme_command_carries_its_name`
`verified-by: bravebot_tui::app::typing_the_exit_command_quits`
`verified-by: bravebot_tui::app::the_loop_command_sends_what_is_left_after_the_interval`


<a id="CMD-5"></a>
### CMD-5: the argument is taken verbatim

Whatever followed the space, spaces and all, with the surrounding whitespace trimmed and nothing
else done to it. A leading `~` is expanded only as a whole first segment, so a directory whose own
name begins with a tilde is not a home-relative path. Nothing shortens it, splits it, or asks the
planner what it meant.

**Why.** The argument is what the command acts on: a directory that becomes trusted, a name a
session is stored under. A person is taken to have endorsed exactly the characters they typed, so
exactly those characters have to arrive.

`verified-by: bravebot_tui::app::the_rename_command_carries_the_whole_name`
`verified-by: bravebot_tui::app::the_add_dir_command_carries_its_directory`
`verified-by: bravebot_tui::app::a_tilde_is_expanded_only_as_a_whole_first_segment`


<a id="CMD-6"></a>
### CMD-6: the set is written down once

One table names every command, its argument and its one-line description, and each name is a
single constant the key handler matches on. The completion list, Tab and the arrows all read the
table, so typing `/` lists every command with what it does and narrowing works on the same set
that dispatches.

**Why.** A word written down in more than one place is a word that is renamed in one of them,
leaving the rest advertising something that no longer works, which a person discovers by typing
it.

`verified-by: bravebot_tui::app::compacting_is_offered_while_a_command_is_being_typed`
`verified-by: bravebot_tui::render::a_slash_offers_every_command_and_what_it_does`


<a id="CMD-7"></a>
### CMD-7: a command name is written in this program, never read from a directory

The set is fixed when this program is built. No name is enumerated from disk, from a project, or
from anything a person installed, and nothing a turn produced can add to it, remove from it, or
change what one of them does.

**Why.** This is what makes the surface small enough to reason about, and it is the reason skills
are kept off it. A skill's name is content: it comes from a directory that may not be trusted, it
is written by whoever wrote the skill, and the rule that an untrusted skill is counted rather than
named exists because such a name can be composed to read like an instruction on a person's screen.
Offering those names in a completion list would put exactly that text in front of the user as
though this program had written it, one keystroke from a line that decides something. If skills
are ever wanted here, the name still may not come from the directory: this clause is what the
change has to answer to.

`/loop` is what that answer looks like. The word is a string literal in this table like every
other command, and the skill it shares a name with is one written into this program too, so
nothing about either came from a directory. A skill somebody installs still has no line here,
whatever it is called, and installing one called `loop` shadows the built-in body without
touching this table.

`verified-by: by-construction (the table is an array of string literals fixed at compile time, and no directory listing, configuration value or turn output reaches it)`
