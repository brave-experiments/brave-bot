# Development

## Building

Requires a recent stable Rust toolchain.

```sh
cargo build
cargo test
make check     # fmt, clippy -D warnings, and tests: what CI enforces
```

`make check-linux` runs the same checks on Linux under the current stable toolchain. Worth
doing before pushing platform-specific code, since a macOS host never compiles the Linux
backend and clippy gains lints between releases.

Reproducible cross-platform binaries are built in a pinned container, so the same artifact
comes out on any host:

```sh
make all-platforms
```

## Releasing

Two steps, because the version is a reviewable change and the tag is the trigger.

```sh
make bump-version BUMP=bugfix   # or minor, major
# commit the result, land it on main
make github-release
```

`bump-version` rewrites the version in `Cargo.toml`, `Cargo.lock`, and `package.json` and stops
there; nothing is committed or pushed for you. `github-release` refuses to tag unless the tree is
clean, the two version files agree, and HEAD is `main` at `origin/main`, then pushes `v<version>`.

The tag push is the only thing that publishes. CI builds all six targets, strips them, writes a
`.sha256` beside each one plus a `SHA256SUMS`, and creates the GitHub release. A tag whose name
disagrees with the version in the tree fails the job rather than publishing assets the installer
would then look for under the wrong name.

Unlike other builds, a tag build does not set `BRAVEBOT_ALLOW_UNCONFIGURED_BUILD`, so a missing
credential fails the release instead of shipping a binary that cannot reach the backend.

Released binaries are **not** code-signed or notarised yet, so macOS Gatekeeper will refuse a
downloaded one until that is added. What makes an unsigned asset safe to fetch is the checksum: the
npm `postinstall` verifies the binary against its published `.sha256` before writing it, so TLS is
not the only thing standing between a substituted asset and an executable. npm publication is
deliberately not wired up.

## Agent configuration

`agents/` is the checked-in source of truth for what an agent reads in this repo: `AGENTS.md`
and the skills under `agents/skills/`. Nothing discovers it there. Claude Code looks under
`.claude/`, and bravebot reads `AGENTS.md` at the workspace root and skills from
`.bravebot/skills`, so a fresh clone links the one source into both:

```sh
make init
```

That creates symlinks and nothing else:

```
.claude/skills/<name>    ->  agents/skills/<name>
.bravebot/skills/<name>  ->  agents/skills/<name>
.claude/CLAUDE.md        ->  agents/AGENTS.md
AGENTS.md                ->  agents/AGENTS.md
```

The links are gitignored, so they are derived state and a skill is written once rather than
copied once per tool. Re-running is idempotent and silent, a stale link is refreshed, and a real
file somebody put in a discovery directory by hand is left alone rather than replaced.
`python3 agents/setup.py list` shows the current state, and `unlink` removes only the links it
owns.

Slash commands and subagents are Claude Code concepts, so `agents/commands/` and `agents/agents/`
link into `.claude/` alone. Neither has to exist: whatever is present is linked, and a directory
added later needs no change to the script.

`make init` does not grant trust. bravebot loads a workspace skill only from a path a person
vouched for ([TRUST-1](specs/trust-map.md)), and a script granting that on your behalf is the
inference that clause forbids, so expect to be asked about `.bravebot/skills` the first time you
start it in this tree.

## Which build wrote a session

Every session record carries the build that produced it, and `bravebot --version` prints the same
string:

```
bravebot 0.1.0 (f2a6e1a, modified)
```

The commit is what the binary was compiled from, and `modified` means the tree had uncommitted
changes at that point. Both matter when reading a transcript back: a session that behaved oddly
is usually being read against code that has moved since, and the alternative to a stamp is
inferring the build from the transcript's own symptoms. Resuming a session recorded by a
different build says so, beside the note about a changed branch.

The stamp is taken by `crates/tui/build.rs`, which watches every crate's sources rather than only
its own, so `modified` cannot go stale while another crate changes underneath it. A build with no
git available says `(no git)` rather than naming a commit it cannot see.

## Configuration

Uses [direnv](https://direnv.net/). Copy the template and fill it in:

```sh
cp .envrc.example .envrc
direnv allow
```

`.envrc` is gitignored and must never be committed, because it holds a signing key.

The build captures whatever is set at build time, so the resulting binary works in any
directory rather than needing direnv wherever it is started. A build with nothing set **fails**,
rather than producing a binary that only works in the tree it came from; to build one
deliberately, set `BRAVEBOT_ALLOW_UNCONFIGURED_BUILD=1` and supply the variables at run time.

The environment still wins when set, which is how a released binary is pointed at a local
backend without rebuilding it. Baked values are masked so `strings` on the binary does not
print them; that is obfuscation and not encryption, so a binary built with a live key should
be treated as holding one.

The cross-build container does not inherit the host environment, so `make all-platforms`
forwards these variables as a BuildKit secret rather than a build argument, which would record
the signing key in the image metadata.

Run `bravebot doctor` to check configuration and confinement without revealing the signing key.

## Words a person reads

Every one of them lives in `crates/i18n/locales/`, one file per locale, and reaches the screen
through `t!(some_message)`. `en-US.ftl` is the reference: it owns the set of messages and the name
and kind of every argument, so a translation can add none of its own and break no call site.

```sh
make locales     # what each translation has of the reference, and what it is missing
```

Adding a language is copying `en-US.ftl` and translating it. No Rust changes, no registration:
the build script finds the file. See
[crates/i18n/locales/README.md](../crates/i18n/locales/README.md), which is written for whoever
is doing the translating rather than for whoever wrote this.

Adding a **message** means adding it to `en-US.ftl` first, because the macro has one arm per
message in the reference and a name no catalog defines does not compile. The other catalogs can
follow later; what they lack is shown in English.

The distinction that matters here is the audience, not the crate. The words the planner reads are
not in a catalog and must not be: a tool's description, the preamble, and the sentence a refused
tool answers with are interface to a model, and rewording them in another language changes what
the agent does. `crates/agent/tests/audience.rs` fails if a catalog lookup appears in one of those
modules. [specs/localization.md](specs/localization.md) is the spec, and its known costs list what
is deliberately left in English.

## Testing the interface

`cargo test` covers the interface a piece at a time: a key press becomes an action, an action is
handled, a screen is drawn. What it cannot reach is the wiring between those pieces, and that is
where the interface bugs have been. `contrib/drive_tui.py` runs a scripted session against a real
terminal so those paths can be exercised, and `contrib/README.md` says how. It needs a backend and
writes real sessions, so it is a tool to reach for deliberately rather than part of `make check`.

## Spec-enforced development

This project is developed against the mini-specs in [specs/](specs/), which are the source of
truth for how it behaves: each clause carries the tests that pin it, and is reviewed
closely by a human before it changes. Code under a spec's `governs` list is reviewed against that
spec rather than on its own, and automation checks that every clause still has coverage.

**A clause ships with the work it describes.** One commit carries the behaviour, the tests, and the
clause that specifies it, in one pull request. Do not land the spec separately from the code: a
clause names the tests that pin it, so a spec commit on its own leaves `make check-spec` pointing at
tests that do not exist yet, and a code commit on its own is behaviour nothing specifies. Reviewing
them together is also the only way to see whether the clause and the code agree.

The exception is a spec written to be argued about before anything is built. That is a design
document, it says `verified-by: none` until the work lands, and the commit should say plainly that
it specifies work not yet done. If you are documenting behaviour that already exists, it is not this
case.

Adding a clause means bumping that spec's clause count in [specs/README.md](specs/README.md);
`make check-spec` fails on a mismatch. Where one commit adds clauses to two specs, that table is
edited by both, so stage it a hunk at a time rather than landing an unrelated count with the wrong
change.

## Reviewing for the rule

Everything here is predicated on one statement: **untrusted content never enters the driver's
context or the planner's**. The *planner* is the model deciding what to do next. The *driver* is
this Rust code, meaning `bravebot-core` and `bravebot-agent` both. Both halves are stated as
clauses in [specs/labels.md](specs/labels.md).

The subtle violations look like safety features, which is what makes them hard to spot. Four
things to look for in a diff:

**1. A branch on untrusted bytes.** The driver may carry untrusted content and hand it to an
effect. It may not branch on it: no `if`, `match`, comparison, or early return whose condition
derives from untrusted bytes.

```rust
// WRONG: the driver decided whether to write, from untrusted bytes.
let text = contents.declassify(&proof);
if text.matches(old).count() > 1 {
    return "error: ambiguous";
}
```

That reads like a careful refusal. It is a decision taken from bytes an attacker may have written,
and the specs refuse it: a decision may be taken only from trusted content. The right move is `Policy::read_trusted_content`, which hands over the
bytes when they are trusted and refuses otherwise, so the untrusted case cannot be taken by
accident.

**2. The same branch, moved into the kernel.** `bravebot-core` is the driver too, so relocating a
decision is not removing it. "It is only for a message to the model" does not help either, because
a message to the model *is* the planner's context.

**3. A `declassify` outside the three gates.** A witness is not permission to inspect. Minting one
records that bytes moved somewhere they were already allowed to go: a filesystem write, an HTTP
body, or a person's screen. Each of those has a gate of its own, `Policy::present`,
`Policy::render_in_place` and `Policy::read_trusted_content`. A `declassify` anywhere
else is almost certainly a violation.

**4. A `Labelled` built by hand.** Never construct one to give a value a better label than its
inputs had. That is laundering, whichever crate it happens in. If a value derived from
untrusted input has to be trusted for something to work, the design is wrong, not the label.

Two places in the kernel do branch on untrusted bytes, deliberately. Both are named under Known
costs in [specs/labels.md](specs/labels.md), because an unlisted exception is indistinguishable
from a violation.
