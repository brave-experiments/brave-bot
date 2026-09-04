---
id: INSTR
title: Resolving standing instructions
status: normative
governs:
  - crates/agent/src/preamble.rs
  - crates/agent/src/home.rs
---

## Scope

Before the planner is asked anything, its context is pre-filled with instructions nobody typed
this turn: `AGENTS.md`, which says how work is done somewhere, and the name and description of
every skill on offer. This spec is about **resolution**: which files are looked for, where, in
what order, and where what they say ends up.

It does not cover what a skill file looks like or what any source is trusted for, which is
[skills.md](skills.md), nor what a label means once assigned, which is [labels.md](labels.md).

## The sources

<a id="INSTR-1"></a>
### INSTR-1: four sources, and no others

| File | Applies to |
|---|---|
| `~/.bravebot/AGENTS.md` | every project |
| `~/.bravebot/skills/<name>/SKILL.md` | every project |
| `<workspace>/AGENTS.md` | this project |
| `<workspace>/.bravebot/skills/<name>/SKILL.md` | this project |

The two roots are spelled differently on purpose: the user's own directory is already `.bravebot`,
so its skills sit directly beneath it, while a project keeps its own out of the way in a dotted
directory rather than at the root where `AGENTS.md` sits.

There is no search of parent directories and no nested `AGENTS.md`. A file at any other path is
an ordinary file, read only when something asks for it by name.

**Why.** A rule that walked upwards would pick up instructions from whatever happened to be above
a project on this machine, which is a different set of instructions on the next machine.

`verified-by: bravebot_agent::preamble::the_home_agents_file_is_read_before_the_project_one`
`verified-by: bravebot_agent::skills::a_workspace_skill_shadows_a_home_skill_of_the_same_name`

<a id="INSTR-2"></a>
### INSTR-2: `~/.bravebot` is the directory the environment names, and there is no fallback

It is `.bravebot` inside the home directory the environment gives. When there is no home, or the
name is empty, there is no user directory and everything kept there is simply absent. Nothing is
guessed and no other location is tried.

**Why.** A fallback would read instructions from a directory the user never chose, and this is
the one place whose contents are trusted for being the user's own. Daemons and containers run
without a home, and everything kept there is optional, so absence is a case to do without rather
than a reason to refuse to start.

`verified-by: bravebot_agent::home::the_home_directory_is_the_one_the_environment_names`
`verified-by: bravebot_agent::home::an_absent_home_is_not_an_error`
`verified-by: bravebot_agent::home::an_empty_home_is_treated_as_no_home_at_all`
`verified-by: bravebot_agent::skills::no_home_directory_is_not_an_error`

<a id="INSTR-3"></a>
### INSTR-3: only the project root is a source, never a directory opened alongside it

A directory opened by name during a session widens where files may be read from. It adds no
standing instructions and no skills, whatever it contains.

**Why.** Opening a directory to read one file out of it would otherwise change how every later
turn behaves, which is not what the person opening it asked for. The project root is what
relative paths mean and what the session is keyed on, and having one answer to "which project is
this" is what keeps that unambiguous.

`verified-by: bravebot_agent::preamble::an_added_directory_contributes_no_standing_instructions`

## What wins

<a id="INSTR-4"></a>
### INSTR-4: sources are read least specific first, so the project has the last word

The user's own directory is read before the project. Both `AGENTS.md` files are read and both
reach the planner, in that order, and a project skill replaces a global one of the same name.
This is the same "most specific wins" rule the trust map uses for paths.

**Why.** A habit carried between projects should hold until the project says otherwise. Shadowing
by name rather than merging is what lets a project override one skill without restating the rest.

`verified-by: bravebot_agent::preamble::the_home_agents_file_is_read_before_the_project_one`
`verified-by: bravebot_agent::skills::a_workspace_skill_shadows_a_home_skill_of_the_same_name`

<a id="INSTR-5"></a>
### INSTR-5: what is resolved goes into the system prompt, never into the conversation

Standing instructions and the catalogue of skill names are put in front of each request as part
of the system prompt. They are not appended to the stored conversation, so a session running many
turns carries one copy of them however long it runs.

**Why.** The system prompt belongs to the build rather than to the conversation. Sending the same
instructions as a message each turn would accumulate a copy per turn, crowding out the task and
paying for the same text repeatedly, and it would leave the planner reading its own conventions
as though a person had just said them.

`verified-by: bravebot_agent::turn::the_preamble_is_not_stored_in_the_conversation`
`verified-by: bravebot_agent::turn::a_trusted_workspace_agents_file_reaches_the_system_prompt`
`verified-by: bravebot_agent::turn::a_workspace_agents_file_is_obeyed_without_anybody_vouching_for_it`

<a id="INSTR-6"></a>
### INSTR-6: a source that is not there is not an error

No `AGENTS.md`, no skills directory, no user directory at all: each is the ordinary case, costs
no notice and no refusal, and offers nothing.

**Why.** Nothing is assumed from silence, so an absent source and an empty one say the same thing.
A warning for the common case is a warning people learn to scroll past, and the times a source
really was dropped are the times that has to be read.

`verified-by: bravebot_agent::turn::a_missing_agents_file_is_not_an_error`
`verified-by: bravebot_agent::skills::a_skills_directory_that_does_not_exist_is_not_an_error`

<a id="INSTR-7"></a>
### INSTR-7: the sources are resolved afresh every turn

Discovery runs per turn rather than once at startup. Writing an `AGENTS.md` or a skill mid-session
takes effect on the next turn, including when the agent wrote it itself. There is nothing to
reload and no session to restart.

**Why.** Reading once at startup would make the file just written the one instruction the planner
cannot see, and the fix for that would be to restart, which loses the conversation.

`verified-by: bravebot_agent::preamble::a_file_written_after_one_turn_is_read_by_the_next`

## Known costs

Accepted deliberately. Do not "fix" one without changing this spec first.

- **Resolution costs a directory listing and up to two file reads every turn.** Cheap next to the
  model call it precedes, and the alternative is a cache that has to be invalidated by something,
  which is a second thing to be wrong about how the filesystem looks.
- **A project cannot turn off a global `AGENTS.md`.** The project's file has the last word, but
  the global one is still in front of the planner and can still be followed where the project
  says nothing that contradicts it. Deleting the global file, or narrowing it, is the only way to
  remove it, since a project is not the right place to be granted power over the user's own
  configuration.
