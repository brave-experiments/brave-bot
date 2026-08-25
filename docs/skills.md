# Skills and standing instructions

Two kinds of file steer a turn before you type anything: **AGENTS.md**, which says how work is
done somewhere, and a **skill**, which says how one kind of task is done.

```
~/.bua/AGENTS.md                          applies to every project
~/.bua/skills/<name>/SKILL.md             available in every project
<workspace>/AGENTS.md                     applies to this project
<workspace>/.bua/skills/<name>/SKILL.md   available in this project
```

Both AGENTS.md files are read, the global one first, so the project's own has the last word. A
project skill of the same name as a global one replaces it, which is the same "most specific
wins" the [trust map](trust.md) uses.

## Writing a skill

A skill is one `SKILL.md` in a directory named after it, opening with frontmatter:

```markdown
---
name: commit-style
description: How commit messages are written here. Use before writing one.
---

Write the subject in the imperative. Explain why in the body, never what.
```

`name` and `description` are both required, and a file missing either is skipped with a note
saying so. Other keys are ignored, so a skill written for another agent works here too.

The **description is what the agent decides from**, so say *when* to use the skill rather than
what it contains. Only the name and the description are put in front of the model; the body is
read when it calls `load_skill`, which is what keeps a directory of long skills from crowding out
the task.

## What is trusted, and why

The name, the description, and the body of a skill all go to the model as instructions, so they
have to come from somewhere nothing hostile can reach.

**`~/.bua` is trusted because it is yours.** It is the directory holding your history and your
sessions, and its contents are ones you put there. That is the same standing the configuration
picking your model and endpoint already has. Nothing is assumed from silence: an empty directory
offers nothing, and the grant is the act of putting a file there.

Stated plainly, because it is the one thing worth knowing before you install anything:

> **A skill you downloaded into `~/.bua/skills` is trusted exactly as far as a config file you
> pasted is.** Read one before you install it. Nothing downstream will second-guess it, because
> everything downstream is built to trust what you vouched for.

**A project's files are trusted only if you said so.** `AGENTS.md` and `.bua/skills` in a working
directory are read through the trust map, so they load when you answered yes at startup and are
**left out entirely** when you did not. They are not quarantined into a reference the way a file
the agent reads is, because a reference to an instruction is no use to anyone: an instruction is
either followed or absent, and one from a directory nobody vouched for has to be absent.

You are told when that happens, once per session:

```
AGENTS.md was not loaded: this directory is not trusted
2 skills in .bua/skills were not loaded: this directory is not trusted
```

Note the second line counts them rather than naming them. A directory in an untrusted project can
be named to read like an instruction, and that name would be on your screen as though the agent
had written it.

## Loading one

```
load_skill name=commit-style   →  the body, as text
```

The name is not a path and never becomes one. It selects from the set found before the turn
started, so a name holding `../` or an absolute path matches nothing and the call is refused:
there is no lookup for it to reach. A name that is merely close to a real one is refused too,
rather than guessed at, since guessing would load instructions nobody asked for.

## Seeing what loaded

Ask, rather than guessing from whether the agent obeyed:

```
/skills          in a session
bua skills       from the command line
```

Both print the same thing: what is on offer, what a project skill shadowed, what was found and
not loaded, and what the whole preamble costs.

```
Skills

  loaded
    commit-style                  .bua/skills/commit-style/SKILL.md
      The PROJECT version, which should shadow the global one.
    project-only                  .bua/skills/project-only/SKILL.md
      Only exists in this project.

  shadowed
    commit-style                  ~/.bua/skills/commit-style/SKILL.md
      replaced by .bua/skills/commit-style/SKILL.md

  not loaded
    .bua/skills/broken/SKILL.md
      it needs a name and a description in its frontmatter

  standing instructions           ~/.bua/AGENTS.md, AGENTS.md
  preamble                        587 bytes, roughly 146 tokens per request
```

**Use `/skills` for the real answer.** A one-shot `bua skills` vouches for nothing, so it runs
with an empty trust map and everything in the project comes back under "not loaded". They are
still listed, by path and size, so you can tell "not trusted" from "not found". Only a session
that you answered yes to shows what that session actually loads.

A skill in an untrusted path is listed by **where it is and how big it is**, never by what it
says about itself:

```
  not loaded
    .bua/skills/broken/SKILL.md
      4 lines, 62 bytes, it is not trusted
```

That is not coyness. Its frontmatter is refused by the same gate that keeps it out of the prompt,
and it would tell you nothing anyway: a skill there is dropped before it is parsed, so the reason
is always this one and never "your frontmatter is wrong". Where the path *is* trusted, the
frontmatter is the diagnosis and it is shown in full.

The token figure is bytes over four. bua does not tokenise locally, which is why the line says
"roughly".

## Commands

`/skills` is the first of them, and a leading `/` in a session now means a command rather than a
prompt. `/help` lists what there is. An unknown command says so instead of being sent, so a typo
costs a line rather than a round trip.

To send a prompt that has to begin with a slash, begin it with two: `//usr/bin is on PATH`
reaches the model as `/usr/bin is on PATH`.

## When changes take effect

Skills and AGENTS.md are looked for afresh **every turn**, so writing one mid-session works, and
so does having the agent write one. There is nothing to reload and no session to restart.
