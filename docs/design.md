# How it works

> The statement under "The rule everything else is predicated on" is load-bearing. Do not
> weaken, qualify, or remove it.

Most agents treat injection as a content problem: scan the text, ask a model whether it looks
malicious, filter it. That approach depends on the model noticing. This one does not. Every
value carries an information-flow label the model never sees and cannot address, and every
consequential action passes through a gate that refuses labels it does not accept. Injected
text is never in a position to be noticed or missed, because it never reaches the components
that decide anything.

## The rule everything else is predicated on

**The driver and the planner NEVER have untrusted content in their context.**

Not "influenced but unable to act on it". Not a matter of degree, or of the model behaving
sensibly. Untrusted content does not enter the driver's context or the planner's context at
all. Every other design decision in this repository follows from that, and anything that
cannot be built while respecting it does not get built.

## The idea

Two axes per value, integrity and confidentiality:

```
L = I × C      I ∈ {T, U}      trusted / untrusted
               C ∈ {pub, priv} public  / private
```

Ordering is a genuine lattice, not a pair of booleans: `(U,priv)` and `(T,pub)` are
**incomparable**. Untrusted input degrades integrity; private input raises confidentiality.
Labels only ever degrade.

Actions declare a role per field:

- **routing** decides where an action goes: a file path, a URL, a recipient. Must be
  `(T,pub)`, derived only from trusted input and never from fetched content.
- **content** is the payload. May be untrusted, and usually is.

That asymmetry is half the mechanism: untrusted text can be carried into an action as content
but can never become routing, so it cannot redirect anything. The other half is that it never
reaches a component that decides. See R1 and R2 below.

## The rules

Each rule names the role it constrains, and each is enforced by a gate that refuses, rather
than by a check a caller could forget to make.

**R1. Nothing untrusted in the planner's context.** Untrusted content is never placed in a
message to the model. It is quarantined in a write-once slot, and the planner is given a
*reference*: origin, line count, byte count, label. The planner acts on content it cannot read
by naming that reference, and the kernel resolves it when the effect fires.

**R2. Nothing untrusted in the driver's context.** The Rust code may carry a `Labelled<T>` and
hand it to an effect, but cannot read one: no `Deref`, `PartialEq`, or `Display`, and no
infallible accessor. Asking for untrusted bytes returns a refusal naming this rule, not a
value. Text is reshaped for presentation inside the kernel, so no tool holds what it formats.

**R3. Decisions may be made only from trusted content.** Comparing text is a decision. On
trusted content that is fine, because a vouched-for path holds nothing an attacker wrote. On
untrusted content it is refused. This is why `edit_file` needs a trusted file: locating a
passage means comparing.

**R4. Routing must be `(T,pub)`.** Where an effect goes, whether a path, a URL, or a recipient,
is derived only from trusted input; untrusted routing is treated as an injection attempt. The
one relaxation: the model may propose a *read* path, because a read changes nothing and is
confined to the workspace.

**R5. Labels only ever degrade.** Integrity may go trusted → untrusted, never the reverse. No
operation anywhere raises a value's integrity.

**R6. Losing trust needs a human.** A write that would make a trusted path untrusted is shown
to the user first, and their approval mints a single-use endorsement bound to that exact path.
It cannot be replayed or redirected.

## Design

- **One network egress path.** A single chokepoint, revalidated on every redirect hop, so a
  permitted host cannot redirect into a denied one.
- **Untrusted work runs in a real sandbox**, meaning OS-level confinement rather than an
  environment allowlist. If confinement cannot be established, the process is not spawned.
  Fails closed.
- **Credentials are brokered.** Sandboxed work never holds an API key or a socket.
- **Extensible via MCP** (HTTP and stdio). No use case is hardcoded. A small set of
  primitives stays native because the kernel needs to label parts of those calls
  individually.

## What the gates look like in practice

Reading a file in a trusted directory, where the content reaches the model:

```
ok      precommit: routing fields ["task"] fixed before any observation
ok      promote: read_file.path proposed by the model, confined and non-destructive
ok      file_read.path [routing] (T,pub)
observe file_read produced (T,priv)
ok      trust: notes.md read as trusted, from a trusted path
ok      render: read_file: content reshaped for presentation, still (T,priv)
ok      present: tool_result: notes.md is (T,priv), so the planner may read it
```

The same read where nothing is vouched for, so the content is quarantined instead:

```
observe file_read produced (U,priv)
ok      trust: notes.md read as untrusted
slot    ref:0 at (U,priv)
ok      present: tool_result: notes.md is (U,priv), quarantined as ref:0; the planner
        sees a reference only
```

## Why some things are absent

A shell is absent, and not by oversight. Unlike a write, a shell command has no separable routing
field to endorse: the string is destination and payload at once, so there is nothing meaningful to
approve. `apply_patch` is excluded for the same reason.

Before adding a tool, ask what its routing field is and whether a human could approve that
field alone.

Running a program passes that test, which is why `run` exists while a shell does not. It takes a
list of argv stages, so the routing field is the argument vector: a person can read it, approve it,
and have it executed exactly as shown. Nothing re-parses it afterwards, so a metacharacter inside an
argument is data rather than syntax.

The distinction is worth stating precisely, because it is easy to read the change as the exclusion
being softened. It was not. The exclusion was about shell strings, and an argv vector is not a shell
string. What a shell would have decided, where one argument ends and the next begins, is decided
here by the caller and shown to the user, so the thing that made a command unapprovable is gone
rather than tolerated.

What keeps it that way is the label on the output rather than any restriction on the program.
Spawned programs are neither confined nor enumerated, since a profile narrow enough to be useful
would break `git push` and `npm install`, and the set of tools a user might ask for is open. So
stdout and stderr are always untrusted and private, which holds without knowing what ran, and every
run is approved because nothing here can establish that a program is harmless. See
[tools.md](tools.md) for the full model.

## Layering

- `bua-core` is the kernel. No I/O, nothing prints. Owns the lattice, the gates, and every
  decision derived from content.
- `bua-agent` holds the tools and the turn loop. Carries labelled values; must not inspect them.
- `bua-tui` and `bua-cli` are presentation. May display released content.
- `bua-net` is the single egress chokepoint. All network traffic passes the policy gate here.
- `bua-mcp`, `bua-sandbox`, `bua-signing`, `bua-config` cover extension, confinement, and auth.

See [tools.md](tools.md) for the per-tool routing and content split, and
[trust.md](trust.md) for the trust map specification.
