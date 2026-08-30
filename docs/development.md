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
