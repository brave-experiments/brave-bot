# brave-user-agent

A coding agent whose defining property is **structural resistance to indirect prompt
injection (IPI)**.

Most agents treat injection as a content problem: scan the text, ask a model whether it looks
malicious, filter it. That approach depends on the model noticing. This one does not. Every
value carries an information-flow label the model never sees and cannot address, and every
consequential action passes through a gate that refuses labels it does not accept. Injected
text can influence *what* a message says; it has no structural path to *where* an action goes.

> Status: early but working. It answers questions about a real workspace, choosing and
> chaining its own tools, with every decision recorded.

## The idea

Two axes per value — integrity and confidentiality:

```
L = I × C      I ∈ {T, U}      trusted / untrusted
               C ∈ {pub, priv} public  / private
```

Ordering is a genuine lattice, not a pair of booleans: `(U,priv)` and `(T,pub)` are
**incomparable**. Untrusted input degrades integrity; private input raises confidentiality.
Labels only ever degrade.

Actions declare a role per field:

- **routing** — where an action goes (a file path, a URL, a recipient). Must be `(T,pub)`.
  Derived only from trusted input, never from fetched content.
- **content** — the payload. May be untrusted, and usually is.

That asymmetry is the whole mechanism. Fetched or model-generated text is quarantined in
write-once slots and can be carried into an action as content, but it can never become
routing — so it cannot redirect anything.

## Design

- **Untrusted content is carryable, not inspectable.** The type system prevents branching on
  untrusted values without an explicit, audited declassification step.
- **One network egress path.** A single chokepoint, revalidated on every redirect hop, so a
  permitted host cannot redirect into a denied one.
- **Untrusted work runs in a real sandbox** — OS-level confinement, not an environment
  allowlist. If confinement cannot be established, the process is not spawned. Fails closed.
- **Credentials are brokered.** Sandboxed work never holds an API key or a socket.
- **Extensible via MCP** (HTTP and stdio). No use case is hardcoded. A small set of
  primitives stays native because the kernel needs to label parts of those calls
  individually.

## Using it

```sh
bua                                  # interactive session
bua "what does src/main.rs do?"      # one-shot
bua "explain this" --file notes.md   # with named context
bua doctor                           # check configuration and confinement
```

In a session: the mouse wheel or Up/Down scrolls, Home/End jumps to either end, Ctrl-T
toggles the audit trail. Add `--trace` to a one-shot run for the same thing:
which gate checked what, the label every value carried, and what was released.

```
ok      precommit: routing fields ["task"] fixed before any observation
ok      promote: read_file.path proposed by the model, confined and non-destructive
ok      file_read.path [routing] (T,pub)
observe file_read produced (U,priv)
ok      release: read_file.contents content released after the action gate
```

### Tools

The model can read files, list them, and search their contents. Each splits into a
**routing** part the gate requires to be trusted and a **content** part that may not be:

| Tool | Routing | Result |
|---|---|---|
| `read_file` | path, offset, limit | contents, untrusted |
| `list_files` | directory, glob | filenames, untrusted |
| `search` | pattern, directory, include glob | matches, untrusted |
| `write_file` | path — **needs your approval** | — |
| `edit_file` | path — **needs your approval** | — |

Filenames are untrusted too — a file can be named to read like an instruction.

Globs and paging exist to keep results small: a large file comes back a page at a time, and
a listing or search can be narrowed to `*.rs` rather than returning a whole tree. A glob is
*routing* — it decides what gets looked at — so an untrusted one is refused like any other
address. Every result is capped, and a capped result says so: silence there would let a
model conclude a file does not exist when the answer was merely cut off. Search matches
literal text rather than a regular expression, since a backtracking pattern arriving through
a turn would be a denial-of-service vector.

The model may choose *which* file to read next, because a read cannot change anything and
is confined to the workspace. Every such choice is recorded as a promotion, so an audit
separates the model's decisions from yours.

A write is different: the wrong file destroys work rather than wasting a step. So the model
never gets to decide one. You are shown the change, and your approval is what mints a
single-use endorsement bound to that exact path — an approval cannot be replayed or
redirected. Where nobody can be asked, such as a one-shot `bua "..."` run, writes are
refused rather than applied unseen.

That approval has to be legible to be worth anything, which is why `edit_file` exists.
Reviewing a whole file body on a terminal is not review, so an edit names the exact passage
to replace and you approve a diff of it. If the passage is missing or occurs more than
once, the edit is refused instead of guessed — a guess would change bytes you were never
shown. Edits are also refused if the file changed after the model read it, since the diff
you approved would no longer describe what happens.

Command execution is absent, and not by oversight. Unlike a write, a shell command has no
separable routing field to endorse: the string is destination and payload at once, so there
is nothing meaningful to approve.

## Building

Requires a recent stable Rust toolchain.

```sh
cargo build
cargo test
```

Reproducible cross-platform binaries are built in a pinned container, so the same artifact
comes out on any host:

```sh
make all-platforms
```

## Configuration

Uses [direnv](https://direnv.net/). Copy the template and fill it in:

```sh
cp .envrc.example .envrc
direnv allow
```

`.envrc` is gitignored and must never be committed — it holds a signing key.

## Credit

The security approach here — enforcing information-flow labels at every boundary, and
separating routing from content so untrusted text cannot redirect an action — comes from
research by **Ali Shahin Shamsabadi**, Senior Privacy Researcher at Brave.

It was developed and demonstrated in
[**SafeHouse**](https://github.com/brave-experiments/safehouse), his research project, which
showed that indirect prompt injection can be made structurally impossible rather than merely
unlikely. This repository reimplements that model for a coding agent; the design is his.

## License

[MPL-2.0](LICENSE)
