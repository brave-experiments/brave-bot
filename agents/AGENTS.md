# bravebot

A general-purpose agent resistant to prompt injection. The guarantee is structural: untrusted
content can be carried and written, but it can never decide what happens.

## The rule that overrides everything

**The driver and the planner NEVER have untrusted content in their context.**

The whole repository is predicated on this. It is not a matter of degree, not a matter of the model
behaving well, and not "influenced but unable to act". Untrusted content does not enter either
context at all.

The driver is the Rust code here, meaning `bravebot-core` and `bravebot-agent` both. The planner is
the model. Neither receives untrusted bytes.

- The driver may **carry** untrusted content and **hand it to** an effect without ever seeing it.
- The driver may **not branch** on it: no `if`, `match`, comparison, or early return whose
  condition derives from untrusted bytes.
- Moving such a branch from `bravebot-agent` into `bravebot-core` does not fix it. `bravebot-core`
  is the driver too, and relocating a decision is not the same as removing it.

Never weaken this statement. If an implementation cannot satisfy it, the implementation is wrong.
Do not restate the rule to match the code.

## Checking a diff against it

[docs/development/reviewing-for-the-rule.md](docs/development/reviewing-for-the-rule.md) is the
review pass: the four shapes a violation takes in a diff, the argument that mistakes a sound design
for one, and the two exceptions that are written down. Read it before writing code that touches a
label, and again before asking anyone to review one.

## Everything else is in the specs

[docs/specs/](docs/specs/README.md) is the source of truth for behaviour, clause by clause, with
the tests that pin each one. Read the spec before changing what it governs. Do not restate its
rules here.

| If the question is about | Read |
|---|---|
| what a label is, who may read what, how one is assigned | [labels.md](docs/specs/labels.md) |
| where an effect may land and what may decide it | [routing.md](docs/specs/routing.md) |
| which paths a person vouched for, and what a write records | [trust-map.md](docs/specs/trust-map.md) |
| rules written in advance about what to ask about or refuse | [permissions.md](docs/specs/permissions.md) |
| the one component that reads untrusted content | [processors.md](docs/specs/processors.md) |
| a tool's arguments, refusals, or results | [tools/](docs/specs/tools/tool-surface.md) |
| fetching a URL, and which hosts a person agreed to | [fetch-url.md](docs/specs/tools/fetch-url.md) |
| why the planner has no shell, and what `run` may do | [shell-mode.md](docs/specs/shell-mode.md), [run.md](docs/specs/tools/run.md) |
| `@`, pasting, dropping a file | [naming-files.md](docs/specs/naming-files.md), [pasting.md](docs/specs/pasting.md), [dropping.md](docs/specs/dropping.md) |
| when a person is asked, and what an answer grants | [prompting.md](docs/specs/prompting.md) |
| answering those prompts in advance: accepting edits, planning, bypassing | [permission-modes.md](docs/specs/permission-modes.md) |
| `AGENTS.md` and skills | [skills.md](docs/specs/skills.md) |
| which crate may do what | [layering.md](docs/specs/layering.md) |
| shortening a long conversation | [compaction.md](docs/specs/compaction.md) |
| what is recorded about every decision | [trace.md](docs/specs/trace.md) |
| planning a whole run before anything is read | [manifest.md](docs/specs/manifest.md) |

Before adding a tool, ask what its routing field is and whether a person could approve that field
alone. If they could not, it does not get built.

## Working here

[docs/development/](docs/development/README.md) is how this repository is worked on: what to run
before a commit and before a push, what one commit contains, the specs the code is developed
against, the security scan, configuration, releasing, and how an issue is labelled.

[docs/best_practices.md](docs/best_practices.md) is what a pull request is reviewed against,
and holds only rules a person has to read a diff to decide. A rule a tool enforces or could enforce
is a check, not an entry there.
