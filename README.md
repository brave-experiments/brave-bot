# brave-user-agent

A coding agent whose defining property is **structural resistance to indirect prompt
injection (IPI)**.

> The statement under "The rule everything else is predicated on" is load-bearing. Do not
> weaken, qualify, or remove it.

Most agents treat injection as a content problem: scan the text, ask a model whether it looks
malicious, filter it. That approach depends on the model noticing. This one does not. Every
value carries an information-flow label the model never sees and cannot address, and every
consequential action passes through a gate that refuses labels it does not accept. Injected
text is never in a position to be noticed or missed, because it never reaches the components
that decide anything.

> Status: early but working. It answers questions about a real workspace, choosing and
> chaining its own tools, with every decision recorded.

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

### Tools

Five tools. Every argument is either **routing**, which decides what the tool touches and must
be `(T,pub)`, or **content**, which is merely carried and may be untrusted:

| Tool | Routing arguments | Content arguments | Result |
|---|---|---|---|
| `read_file` | `path`, `offset`, `limit` | none | the lines, or a reference |
| `list_files` | `directory`, `pattern` | none | the paths, or a reference |
| `search` | `pattern`, `directory`, `include` | none | matching lines, or a reference |
| `write_file` | `path` | `contents` | confirmation |
| `edit_file` | `path`, `replace_all` | `old_text`, `new_text` | confirmation |

Reads return the content itself when it is trusted and a reference when it is not, per R1.
Writes are silent or shown according to the trust table below, per R6.

`read_file` pages: it caps at 500 lines and 2000 characters per line, reports the range it
returned, and gives the offset to continue from. A file that is not text is reported as binary
rather than as a decoding error.

`list_files` and `search` take a glob (`*.rs`, `src/**/*.rs`; `*` and `?` do not cross `/`,
`**` does, brace groups are unsupported) and skip version-control and build directories. Both
cap their output and **say so when they do**. Silence there would let the model conclude a file
does not exist when the answer was merely cut off.

`search` matches a literal substring, not a regular expression: a backtracking pattern arriving
through a turn would be a denial-of-service vector. The glob matcher is hand-written and
non-backtracking for the same reason.

`edit_file` replaces an exact passage and refuses rather than guessing when that passage is
missing or occurs more than once, since a guess would change bytes nobody reviewed. It also
refuses if the file changed since it was read, and it requires a **trusted** file, because
locating a passage means comparing text (R3). Use `write_file` for an untrusted file: nothing
is located, and the body is shown in full.

Filenames are content too, since a file can be named to read like an instruction, so an
untrusted listing is quarantined exactly as file contents are. A listing or search that
touches several files is trusted only if every one of them is.

The model may choose *which* file to read next, because a read cannot change anything and
is confined to the workspace. Every such choice is recorded as a promotion, so an audit
separates the model's decisions from yours.

A write is different: the wrong file destroys work rather than wasting a step. So the model
never gets to decide one. Your approval is what mints a single-use endorsement bound to that
exact path, so an approval cannot be replayed or redirected. Where nobody can be asked, such
as a one-shot `bua "..."` run, writes are refused rather than applied unseen.

### Trusted directories

At startup you are asked whether you trust the working directory. Trusting it means files
there are read as **trusted**, which is what lets ordinary work proceed without a prompt for
every edit. Decline and nothing is trusted, so every write is shown to you.

Trust is per path, and the **most specific rule wins**, so a trusted project can contain an
untrusted subtree, and that subtree can contain a trusted path again.

A prompt asks you one thing: **may this path stop being trusted?** That is the only
consequence a later step cannot undo, since a path recorded as untrusted can no longer be
examined or edited.

| data | destination | prompt? | effect on the trust map |
|---|---|---|---|
| trusted | trusted | no | unchanged |
| untrusted | trusted | **yes** | that path becomes untrusted |
| trusted | untrusted | no | that path becomes trusted |
| untrusted | untrusted | no | unchanged |
| either | *never mentioned* | **yes** | that path takes the data's trust |

Writing trusted data never asks. For data to be trusted the turn must have observed nothing
untrusted, so there is no attacker-influenced byte in it, and the destination only gains trust,
never loses it.

The last row is why a path nobody has mentioned differs from one you deliberately marked
untrusted: the first has no decision behind it, so the first write there is the moment to ask.
This is also what makes declining at startup meaningful: with nothing vouched for, every write
is shown.

The second row is what closes the obvious hole. If untrusted data, meaning anything derived
from the web or from a file outside a trusted path, is written into a trusted directory, that
file is recorded as untrusted. Reading it back returns untrusted data. Otherwise a round trip
through the filesystem would launder injected text into trusted input, and the trust map would
become a bypass for the gate it exists to support.

Marking is always per file, never per directory: one untrusted file does not taint its
siblings.

When a write is shown, it has to be legible to be worth anything, which is why `edit_file`
exists. Reviewing a whole file body on a terminal is not review, so an edit names the exact
passage to replace and you approve a diff of it. If the passage is missing or occurs more than
once, the edit is refused instead of guessed, because a guess would change bytes you were never
shown. Edits are also refused if the file changed after the model read it, since the diff
you approved would no longer describe what happens.

`edit_file` only works on trusted files. Finding the passage means comparing text, and that is
a decision. On untrusted content it would let file contents decide whether an effect happens.
So an untrusted file is refused rather than edited blind; trust the path, or replace the whole
file with `write_file`, where nothing is located and the body is shown to you in full.

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

`.envrc` is gitignored and must never be committed, because it holds a signing key.

## Credit

Ali Shahin Shamsabadi and Brian R. Bondy developed the idea behind this project: that
indirect prompt injection can be made structurally impossible rather than merely unlikely, by
enforcing information-flow labels at every boundary and separating routing from content so
untrusted text cannot redirect an action.

Ali took the idea considerably further, working out the enforcement model in detail and
building the first prototype of it in
[SafeHouse](https://github.com/brave-experiments/safehouse). This repository applies that model
to a coding agent.

The model backend is [brave/aichat](https://github.com/brave/aichat). The client-side handling
it builds on comes from [brave/brave-core](https://github.com/brave/brave-core). The dockerized
reproducible build setup is from [bbondy/guardrails](https://github.com/bbondy/guardrails).

## License

[MPL-2.0](LICENSE)
