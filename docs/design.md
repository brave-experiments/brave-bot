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
by naming that reference, and the kernel resolves it when the effect fires. Where the content
has to be *changed* rather than moved, it goes to a processor: see below.

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

## The one reader: processors

R1 leaves a gap. An agent that may not read a file also cannot change it, because locating a
passage is a comparison (R3) and writing a whole new body would mean the planner authoring text
for a file it never saw. Answering questions about an untrusted repository is useful; being
unable to do any work in one is not.

A **processor** is what closes it. A second model instance is started with no tools, no
conversation, no memory of the session and no workspace, given exactly the slots the driver
names, and asked to produce text. That text is not returned to the planner: it goes into a new
slot at the label taint gives it, and the planner receives another reference.

The gap closes without anything being relaxed, because a processor is neither the driver nor the
planner and it decides nothing. Its output cannot become routing, since it is quarantined. Its
label is fixed before it runs, from its inputs, so nothing it writes can improve how what it
writes is labelled. It cannot act, because there is no tool in the request. It cannot persist,
because there is no second round.

So the worst an injected line can achieve, having reached the one component that can read it, is
different bytes in a slot nobody has read, going to a path the user approved.

The confinement here is the capability set rather than an OS boundary: there is no untrusted
*code* involved, only untrusted *content*, and the caller is the same trusted driver as
everywhere else.

## Compacting a conversation

Each round re-sends the whole history, so a session that runs long enough is refused by the
server. Compaction replaces the older part of the exchange, in the request only, with a summary
of it.

The summariser is a **planner-context call, not a processor**, and the difference is the whole of
why it is sound. A processor is the one component allowed to read untrusted content, and the price
is that everything it produces is quarantined: what it could make here is a summary the planner may
not read, which is not a summary the planner can carry on from. What licenses this call instead is
the opposite property. Every message in a conversation has already been past `present`: either the
kernel judged it trusted and showed it, or what went in was a reference and the bytes stayed in
quarantine. So a model given that exchange is given exactly what the planner was given, and R5
holds because `label_model_output` labels the answer from the context rather than upgrading
anything.

Where the context has gone untrusted, the summary is untrusted, and there is nowhere for it to go:
quarantining it would hand the planner a reference to its own history. `Policy::adopt_summary`
refuses, and the session carries on with the conversation it had. That is longer than anyone
wanted and is the one outcome here that is never wrong.

Nothing else moves. The quarantine is untouched, so a reference handed out earlier still resolves;
the reference counter does not rewind, since slots are written once; the integrity does not
recover, since compaction is not a fresh session. And what is shortened is the request, not the
record: the replaced messages are still the person's own transcript and are still read back to
them.

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

Changing that same file, which nothing along the way is able to read:

```
ok      reference: spawn_processor.reads names ref:1
ok      processor: processor over ref:1 reads ref:1 and writes (U,priv), with no tools,
        no memory and nothing to write but that one slot
ok      processor: input assembled from 1 slot(s) inside the kernel
ok      processor: output labelled (U,priv) by taint over its inputs
slot    ref:3 at (U,priv)
ok      present: tool_result: quarantined as ref:3; the planner sees a reference only
ok      resolve: write_file: ref:3 resolved to its quarantined content, (U,priv)
release ref:3 (U,priv) -> (U,pub)
ok      declassify: ref:3 released into src/config.py, which is inside the workspace
ok      approval: src/config.py: a path nobody has vouched for either way, asking
```

## Why some things are absent

A shell is absent **from the agent**, and not by oversight. Unlike a write, a shell command has no
separable routing field to endorse: the string is destination and payload at once, so there is
nothing meaningful to approve. `apply_patch` is excluded for the same reason.

The qualification matters, and it is not a softening. The exclusion exists because a person cannot
approve a routing field that a shell string does not have, and the reason they would need to is that
the string came from the planner, which an attacker may have steered into producing it. A line the
**user** typed raises neither question: there is no routing to endorse separately because the person
who would have been asked to endorse it wrote the whole thing. So shell mode, where a user types
`!` and then a command line, runs it through their own `$SHELL`, globs and redirection and all. See
[tools.md](tools.md#shell-mode).

What must stay true is the thing the exclusion was always aimed at: the planner has no shell and
cannot obtain one. `run` remains argv-only, `bravebot-agent::exec` never builds a command line, and
nothing a model wrote, nothing read out of a file, and nothing a processor produced may reach
`bravebot-agent::shell`. Its justification is the provenance of the string, which no gate can check
from the bytes and which only the call site can vouch for.

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
stdout and stderr are always untrusted and private, which holds without knowing what ran, and a run
is approved by a person because nothing here can establish that a program is harmless.

A person may answer that question once per command rather than once per run, by pressing `a` at the
prompt. That records the resolved binary and its exact arguments on the session, and it grants two
things together: the command runs unasked, and what it prints is trusted so the model may read it.

The second is a human assertion rather than an inference, and it is admissible for the same reason
the trust map is: a directory's contents are trusted because the user said so, not because anything
inspected them. Nothing infers it, and an entry covers one command rather than one program, so
`git log` says nothing about `git push`. It is still not an allowlist and does not decide what may
run. See [tools.md](tools.md) for the full model.

## Layering

- `bravebot-core` is the kernel. No I/O, nothing prints. Owns the lattice, the gates, and every
  decision derived from content.
- `bravebot-agent` holds the tools and the turn loop. Carries labelled values; must not inspect
  them.
- `bravebot-tui` and `bravebot-cli` are presentation. May display released content.
- `bravebot-net` is the single egress chokepoint. All network traffic passes the policy gate here.
- `bravebot-mcp`, `bravebot-sandbox`, `bravebot-signing`, `bravebot-config` cover extension,
  confinement, and auth.

See [tools.md](tools.md) for the per-tool routing and content split, and
[trust.md](trust.md) for the trust map specification.
