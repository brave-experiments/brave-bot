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

Add `--trace` to a one-shot run, or press Ctrl-T in a session, to see the audit trail:
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
| `read_file` | path | contents, untrusted |
| `list_files` | directory | filenames, untrusted |
| `search` | pattern, directory | matches, untrusted |

Filenames are untrusted too — a file can be named to read like an instruction.

The set is deliberately read-only. A write or command destination chosen by the model
would be routing derived from whatever it just read, which is the attack this prevents.
The model may propose *which* file to read next, because that operation cannot change
anything and is confined to the workspace; every such choice is recorded as a promotion so
an audit separates the model's decisions from yours.

## Building

Requires a recent stable Rust toolchain.

```sh
cargo build
cargo test
```

Reproducible cross-platform binaries are built in a pinned container, so the same artifact
comes out on any host:

```sh
make build-all
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

It was developed and demonstrated in **SafeHouse**, his research project, which showed
that indirect prompt injection can be made structurally impossible rather than merely
unlikely. This repository reimplements that model for a coding agent; the design is his.

(SafeHouse is currently a private repository, so it is not linked here.)

## License

[MPL-2.0](LICENSE)
