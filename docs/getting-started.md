# Getting started

Installing bravebot, running it, and what it asks you before and during work on a repository.

## Install

```sh
npm install -g @brave/bravebot
```

This downloads the release binary for your platform and verifies its checksum. macOS, Linux,
and Windows on both x86_64 and arm64 are supported. To build from source instead, see
[development.md](development.md).

## Using it

```sh
bravebot                                  # interactive session
bravebot "what does src/main.rs do?"      # one-shot
bravebot "explain this" --file notes.md   # with named context
bravebot doctor                           # check configuration and confinement
```

The prompt has the editing keys you would expect, Shift-Enter (or Ctrl-J) for a new line without
sending, Ctrl-V for pasting including screenshots, Ctrl-G to compose in `$EDITOR`, and Ctrl-T for
the audit trail. Long pastes and dropped files fold to a marker you can delete. See
[specs/terminal-input.md](specs/terminal-input.md) for the full set and the terminal quirks behind a
couple of them.

Type `!` on an empty prompt and the line becomes a command for your own shell, so `! cargo test`
runs in `$SHELL` with globs and redirection intact. What it prints goes to the model in full, which
is what lets you follow it with "fix the first failure". See
[specs/shell-mode.md](specs/shell-mode.md) for what that means for trust.

## Slash commands

A line that is exactly a word beginning with `/` is acted on here rather than sent as a prompt.
`/model` chooses which model to think with. `/theme` opens a centred panel over the session and
live-previews as you move; Enter keeps the choice, Escape keeps what you had. `/theme nord`
applies a named theme without opening the panel. The choice is stored in `~/.bravebot` and applies
in every directory. Custom themes are JSON files under `~/.bravebot/themes/`. See
[specs/commands.md](specs/commands.md) for what makes a line a command, and
[specs/terminal-transcript.md](specs/terminal-transcript.md) for how themes paint the interface.

Add `--trace` to a one-shot run for the audit trail: which gate checked what, the label every
value carried, and what was released.

## Language

bravebot reads the interface in your language where a translation for it has shipped, and in
English otherwise. It takes the first of `BRAVEBOT_LOCALE`, `LC_ALL`, `LC_MESSAGES` and `LANG`
that is set, so on a machine already set up for French there is nothing to do.

```sh
bravebot                          # whatever your shell says
BRAVEBOT_LOCALE=fr bravebot       # this once
export BRAVEBOT_LOCALE=fr         # from now on
```

`BRAVEBOT_LOCALE` is there so one program can be in a language the rest of the shell is not,
which is usually wanted the other way round: an English interface on an otherwise French machine.

`fr-CA` and `fr-BE` are answered by the French catalog where they have none of their own, and a
language nothing has shipped for reads in English. `LC_ALL=C` asks for no translation at all.

English and French are what ship today. Adding a language is a file, and needs no Rust:
[crates/i18n/locales/README.md](https://github.com/brave-experiments/brave-bot/blob/main/crates/i18n/locales/README.md).

What stays in English whatever you set: the names of the slash commands, so `/model` is `/model`
everywhere; the letters a question is answered with, `y` and `n`; and the audit trail, which is a
record rather than prose. Nothing the model is sent changes with your language either, so
switching it changes what you read and never what the agent does.

## What is trusted, and how it gets that way

Nothing asks you anything at startup. The directory you started in is somewhere to work: the agent
can list it and write in it, and that says nothing about what is in any of its files.

A file becomes readable when something stands behind it. Usually that is a checker: the first time
a turn needs a file nobody has vouched for, the whole of it goes to an isolated model with no
tools and no memory, which answers only whether the content is what it was said to be and whether
it carries anything addressed to whoever reads it. A clean verdict is recorded for that file and
the turn carries on. Anything else leaves the file quarantined, and the agent works on it through
a reference without ever being shown it.

You can also stand behind a file yourself by naming it with `@path` or dropping it on the window,
which skips the check.

`--disable-vetting` turns the checker off, and then a file nobody vouched for is simply
quarantined. `/status` lists every rule in force and where each came from.

The rules, and every other way a path comes to be trusted, are in
[specs/trust-map.md](specs/trust-map.md).

## Skills and AGENTS.md

Put standing instructions in `AGENTS.md` and they apply to every task in that directory. Put a
skill in `~/.bravebot/skills/<name>/SKILL.md` and it is available in every project:

```markdown
---
name: commit-style
description: How commit messages are written here. Use before writing one.
---

Write the subject in the imperative. Explain why in the body, never what.
```

Only the name and the description are put in front of the model, which loads the body when the
task calls for it. Your own `~/.bravebot` is trusted for being yours, and a project's `AGENTS.md`
and `.bravebot/skills` are recorded when a session opens, so they load without your being asked.
That last part cuts both ways: standing instructions in a repository you cloned are still standing
instructions, so read them the way you would read its build script. See
[specs/skills.md](specs/skills.md).

## Configuration

Configuration is built into the released binary, so there is nothing to set up. `bravebot doctor`
reports what it will use. To point it at a different backend, see
[development.md](development.md#configuration).
