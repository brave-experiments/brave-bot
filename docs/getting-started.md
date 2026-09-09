# Getting started

Installing bravebot, running it, and what it asks you before and during work on a repository.

## Install

```sh
npm install -g @brave/bravebot
```

This downloads the release binary for your platform and verifies its checksum. macOS, Linux,
and Windows on both x86_64 and arm64 are supported.

On macOS and Linux there is an install script, for a machine with no npm on it:

```sh
curl -fsSL https://raw.githubusercontent.com/brave-experiments/brave-bot/main/install.sh | sh
```

It fetches the newest release for your platform, checks it against the published checksum, and
puts it in `/usr/local/bin`, asking for sudo only if that directory is not yours to write to.
`INSTALL_DIR` puts it somewhere else:

```sh
curl -fsSL https://raw.githubusercontent.com/brave-experiments/brave-bot/main/install.sh | INSTALL_DIR="$HOME/.local/bin" sh
```

Running the line again is how an install made this way is updated: it remembers where the last
one went, so a second run replaces that binary rather than leaving two on your PATH.

To build from source instead, see [development.md](development.md).

## Staying current

A session started on a version that has been superseded says so, once, under the trust question,
and gives the line that updates the copy you are running: the npm command for an npm install and
the script again for a script install. A build from source is left alone, since neither line would
update one.

Nothing about that waits: the notice comes from an answer written down on an earlier launch, and
the request that refreshes it, at most one a day, runs behind the session and is for the next one.
A machine that is offline, or a first run with nothing recorded yet, says nothing. An incognito
session neither records an answer nor asks for one, and a build from source never asks at all.

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
in every directory.

Schemes published for both a light and a dark terminal are one row, such as `gruvbox`, and follow
whichever background is sensed at startup. Where that guess is wrong, name the half you want:
`/theme gruvbox-dark` and `/theme gruvbox-light` stay put. Custom themes are JSON files under
`~/.bravebot/themes/`, and an ink there may be a pair, `{"dark": "#282828", "light": "#fbf1c7"}`,
to follow the terminal the same way. See
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
