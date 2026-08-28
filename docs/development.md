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

## Conventions

- **Never use an em-dash**, anywhere: not in documentation, commit messages, code comments, or
  pull requests. A comma, a colon, a semicolon, parentheses, or two sentences will do.
- Comments explain **why**, never what. Prefer no comment to a restatement of the code.
- Tests are behavioural and named as sentences. A doc comment on a test says why the property
  matters, not what the test does.
- Test refusals and denials, not just happy paths. A test that would pass against the buggy
  code is worthless: verify a new test fails before the fix.
- Small commits, one property each. Run `make check` before committing.
- No new dependencies without a reason that survives scrutiny. Patterns that arrive through a
  turn are attack surface: prefer literal matching and hand-written, non-backtracking matchers
  to a regex engine.

## Reviewing for the rule

[The rule](design.md#the-rule-everything-else-is-predicated-on) is what the whole repository is
predicated on, and the subtle violations look like safety features. The driver may **carry**
untrusted content and hand it to an effect, but it may not **branch** on it: no `if`, `match`,
comparison, or early return whose condition derives from untrusted bytes.

```rust
// WRONG: the driver decided whether to write, from untrusted bytes.
let text = contents.declassify(&proof);
if text.matches(old).count() > 1 {
    return "error: ambiguous";
}
```

Moving such a branch from `bravebot-agent` into `bravebot-core` does not fix it. `bravebot-core`
is the driver too, and relocating a decision is not the same as removing it. Nor does "it is
only for a message to the model", which is R1.

A declassification witness is **not permission to inspect**. Minting one records that bytes
moved somewhere they were already allowed to go: a filesystem write, an HTTP body, or a human's
screen. The three legitimate destinations have gates of their own, `Policy::present`,
`Policy::render_in_place`, and `Policy::read_trusted_content`. A `declassify` call outside those
is almost certainly a violation.

Never construct a `Labelled` by hand to give a value a better label than its inputs had. That
is laundering, whichever crate it happens in. If a value derived from untrusted input needs to
be trusted for something to work, the design is wrong, not the label.
