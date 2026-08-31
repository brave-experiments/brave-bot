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

## Trusted directories

At startup you are asked whether you trust the working directory. **Trust it** and ordinary work
proceeds without a prompt for every edit. **Decline** and nothing is trusted, so every write is
shown to you first.

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
task calls for it. Your own `~/.bravebot` is trusted for being yours; a project's `AGENTS.md`
and `.bravebot/skills` are read through the trust map, so they load when you vouched for the
directory and are left out when you did not. See [specs/skills.md](specs/skills.md).

## Configuration

Configuration is built into the released binary, so there is nothing to set up. `bravebot doctor`
reports what it will use. To point it at a different backend, see
[development.md](development.md#configuration).
