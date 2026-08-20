# brave-user-agent

A coding agent whose defining property is **structural resistance to indirect prompt
injection (IPI)**.

Most agents treat injection as a content problem: scan the text, ask a model whether it looks
malicious, filter it. That approach depends on the model noticing. This one does not. Every
value carries an information-flow label the model never sees and cannot address, and every
consequential action passes through a gate that refuses labels it does not accept. Injected
text can influence *what* a message says; it has no structural path to *where* an action goes.

> Status: early. The security kernel is being built before the agent that sits on top of it.

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

## License

[MPL-2.0](LICENSE)
