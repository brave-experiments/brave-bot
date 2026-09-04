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

## Reviewing for it

The subtle violations look like safety features.

```rust
// WRONG: the driver decided whether to write, from untrusted bytes.
let text = contents.declassify(&proof);
if text.matches(old).count() > 1 {
    return "error: ambiguous";
}
```

```rust
// ALSO WRONG: relocating the same branch into bravebot-core does not fix it.
// And "it is only for a message to the model" does not either: that is the planner's context.
messages.push(Message::user(format!("Contents:\n{}", text)));
```

**A witness is not permission to inspect.** Minting one records that bytes moved somewhere they
were already allowed to go: a filesystem write, an HTTP body, or a human's screen. Those three
destinations have gates of their own, `Policy::present`, `Policy::render_in_place` and
`Policy::read_trusted_content`. A `declassify` call outside them is almost certainly a violation.

Never construct a `Labelled` by hand to give a value a better label than its inputs had. That is
laundering, whichever crate it happens in. If a value derived from untrusted input has to be
trusted for something to work, the design is wrong, not the label.

Three places in the kernel do branch on untrusted bytes, deliberately. All three are named under
Known costs in [docs/specs/labels.md](docs/specs/labels.md). An unlisted exception is
indistinguishable from a violation.

## Everything else is in the specs

[docs/specs/](docs/specs/README.md) is the source of truth for behaviour, clause by clause, with
the tests that pin each one. Read the spec before changing what it governs. Do not restate its
rules here.

| If the question is about | Read |
|---|---|
| what a label is, who may read what, how one is assigned | [labels.md](docs/specs/labels.md) |
| where an effect may land and what may decide it | [routing.md](docs/specs/routing.md) |
| which paths a person vouched for, and what a write records | [trust-map.md](docs/specs/trust-map.md) |
| the one component that reads untrusted content | [processors.md](docs/specs/processors.md) |
| asking whether quarantined content is what it was said to be | [vetting.md](docs/specs/vetting.md) |
| a tool's arguments, refusals, or results | [tools/](docs/specs/tools/tool-surface.md) |
| why the planner has no shell, and what `run` may do | [shell-mode.md](docs/specs/shell-mode.md), [run.md](docs/specs/tools/run.md) |
| `@`, pasting, dropping a file | [naming-files.md](docs/specs/naming-files.md), [pasting.md](docs/specs/pasting.md), [dropping.md](docs/specs/dropping.md) |
| when a person is asked, and what an answer grants | [prompting.md](docs/specs/prompting.md) |
| `AGENTS.md` and skills | [skills.md](docs/specs/skills.md) |
| which crate may do what | [layering.md](docs/specs/layering.md) |
| shortening a long conversation | [compaction.md](docs/specs/compaction.md) |
| what is recorded about every decision | [trace.md](docs/specs/trace.md) |
| planning a whole run before anything is read | [manifest.md](docs/specs/manifest.md) |

Before adding a tool, ask what its routing field is and whether a person could approve that field
alone. If they could not, it does not get built.

## Committing

No co-attribution markers for Claude Code.

**`make check` must pass before every commit.** Not after it, not in the next one, and not
"probably fine". It runs fmt, clippy with `-D warnings`, and the tests, and a commit made without
it is a broken state that somebody else finds later, from the history, which is the worst place
to find one. There is no exemption for a change that only touched a comment, a document, or a
name: fmt and clippy fail on those as readily as on anything else. If a check cannot pass for a
reason outside the change, say so in the commit message rather than leaving it to be discovered.

Run it on its own and read what it exits with. Piped into a filter, the status you see belongs to
the filter: `make check | grep error` reports success on a formatting failure, because a fmt diff
says nothing matching that pattern and grep exited happily. `make init` installs a pre-commit hook
that refuses a commit failing fmt or clippy, but it stops there. The tests are too slow to run on
every commit, so nothing but running `make check` tells you about those.

**One change per commit, with its tests in that same commit.** A commit is the unit somebody
reads, reverts, and bisects on, so it has to stand up alone: the change, the tests that pin it,
and any documentation the change makes wrong if it lands without it. Tests that arrive a commit
later say the behaviour went in unverified, and a bisect that lands between the two hits a
revision passing for the wrong reason.

Keep them small. If the message needs an "and" to describe what the commit does, it is usually
two commits. Every commit must leave the tree building and passing, since that is the whole of
what makes a history worth bisecting.

**A spec clause ships with the work it describes**, in the same commit and the same pull request.
Do not land the spec on its own. A clause names the tests that pin it, so a spec commit by itself
leaves `make check-spec` pointing at tests that do not exist, and a code commit by itself is
behaviour nothing specifies. Reviewing them together is also the only way to see whether the clause
and the code actually agree. The history has commits that did it the other way; do not read those
as the convention.

The exception is a spec written to be argued about before anything is built. That is a design
document, its clauses say `verified-by: none` until the work lands, and the message should say
plainly that it specifies work not yet done. Documenting behaviour that already exists is not this
case.

Adding a clause means bumping that spec's count in [docs/specs/README.md](docs/specs/README.md),
and `make check-spec` fails on a mismatch. When one commit adds clauses to two specs, that one table
is edited by both, so stage it a hunk at a time rather than landing an unrelated count alongside the
wrong change.


## Pull requests

No co-attribution markers for Claude Code or other tools.

## Conventions

- **Never use an em-dash.** Not in documentation, commit messages, the README, code comments,
  pull requests, or anywhere else. Reword instead: a comma, a colon, a semicolon, parentheses,
  or two sentences will always do the job.
- Comments explain **why**, never what. Prefer no comment to a restatement of the code.
- **Never write about your own process.** Not in a commit message, a spec, a comment, a pull
  request, or a reply. "I was wrong earlier", "as I said above", "this corrects what I claimed",
  "I initially thought", and narration of what was checked, guessed or assumed are all noise.
  Nobody reading this later shares the conversation it came from.

  State what is true about the code, in the present tense, as though saying it for the first time.
  Where a correction matters, the corrected fact is the whole of it: write "an absent store reports
  nothing and the turn runs on the free tier", not "I said it warns, but it does not". This applies
  most where it is most tempting, which is immediately after getting something wrong.
- Tests are behavioural and named as sentences. A doc comment on a test says why the property
  matters, not what the test does.
- Test refusals and denials, not just happy paths. A test that would pass against the buggy
  code is worthless: verify a new test fails before the fix.
- Specs live in docs/specs and are the source of truth for behaviour. Before writing or changing
  one, read docs/specs/README.md: it is the specification for specs, covering clause ids, the
  front matter, coverage, and how a spec is allowed to refer to anything outside itself. Follow it
  rather than the shape of whatever spec you happen to be editing.
- No new dependencies without a reason that survives scrutiny. Patterns that arrive through a
  turn are attack surface: prefer literal matching and hand-written, non-backtracking
  matchers to a regex engine.
