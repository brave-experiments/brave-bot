# bua

A coding agent resistant to prompt injection. The guarantee is structural: untrusted content
can be carried and written, but it can never decide what happens.

## The rule that overrides everything

**The driver and the planner NEVER have untrusted content in their context.**

The whole repository is predicated on this. It is not a matter of degree, not a matter of the
model behaving well, and not "influenced but unable to act". Untrusted content does not enter
either context at all.

The driver is the Rust code here. The planner is the model. Neither receives untrusted bytes.

- The driver may **carry** untrusted content (`Labelled<String>`) and **hand it to** an effect
  without ever seeing it.
- The driver may **not branch** on untrusted content: no `if`, `match`, comparison, or early
  return whose condition is derived from untrusted bytes.
- Moving such a branch from `bua-agent` into `bua-core` does not fix it. `bua-core` is the
  driver too. Relocating a decision is not the same as removing it.

Never weaken this statement. If an implementation cannot satisfy it, the implementation is
wrong. Do not restate the rule to match the code.

`Labelled<T>` enforces the mechanical half at compile time: no `Deref`, `PartialEq`, or
`Display`, and no infallible accessor. Reading requires a `Declassification` witness only the
policy layer can mint.

**A witness is not permission to inspect.** Minting one does not make reading untrusted content
acceptable; it records that bytes moved somewhere they were already allowed to go, such as a
filesystem write, an HTTP body, or a human's screen. The three legitimate destinations have gates
of their own:

- `Policy::present` decides whether the planner sees content or a reference.
- `Policy::render_in_place` reshapes content for presentation without exposing it.
- `Policy::read_trusted_content` hands over bytes only when they are trusted, and **refuses**
  otherwise.

If you are calling `declassify` outside those, you are almost certainly adding a violation.

Watch for this in review. The subtle violations look like safety features:

```rust
// WRONG: the driver decided whether to write, from untrusted bytes.
let text = contents.declassify(&proof);
if text.matches(old).count() > 1 {
    return "error: ambiguous";
}
```

```rust
// ALSO WRONG: relocating the same branch into bua-core does not fix it.
// And "it is only for a message to the model" does not either. That is R1.
messages.push(Message::user(format!("Contents:\n{}", text)));
```

## Trusted content may be examined; untrusted content may not

The rule bans deciding from **untrusted** content. Trusted content carries no such
restriction, since it came from a path the user vouched for, so comparing it decides nothing an
attacker can steer. `Policy::read_trusted_content` is the gate: it returns the bytes if they
are trusted and refuses otherwise, so a caller cannot quietly take the untrusted case.

This is why `edit_file` requires a trusted file. Locating a passage is a comparison, so on an
untrusted file it is refused rather than performed.

Integrity is the only axis that matters for this. Workspace content is private as a matter of
course, and examining it in-process releases nothing.

## Processors: the one thing that reads untrusted content

The rule names the driver and the planner. A **processor** is neither, and it exists because an
agent that may not read a file also cannot change it: `edit_file` refuses on an untrusted file,
and `write_file` would need a body the planner could only have guessed.

A processor is a second model instance holding **no capabilities at all**. No tools, no
conversation, no memory of the session, no workspace, no spawn. It is given the slots its spec
names and returns text, and that text goes straight into a new slot at the label taint gives it.
The planner gets a reference, exactly as it would for a file it may not read.

So untrusted content reaching a processor can do one thing: change the bytes in a slot nobody
has read. It cannot redirect an effect, because nothing it produces can reach a routing field.

The properties this rests on, none of which may be relaxed:

- **The spec is built by the driver and frozen before the run.** `Policy::before_processor` is
  the only thing that constructs a `ProcessorSpec`, and nothing widens one afterwards.
- **The output label is computed before the processor runs**, by taint over the inputs. Nothing
  the processor writes has any say in how what it writes is labelled.
- **The input is assembled inside the kernel.** `Policy::compose_processor_input` concatenates
  the slots; the driver carries the result wrapped and hands it to the call.
- **No tools, ever.** The request carries no tool list. A processor with one tool is a second
  planner with untrusted content in its context, which is the thing this design refuses.
- **One call, no loop.** There is no round for a reply to steer.
- **The output is never shown to the planner.** It is presented like any other untrusted
  content: a reference, and nothing else.

The confinement is the capability set, not an operating system boundary. `bua-sandbox` confines
processes running code we did not write; a processor's caller is our own code, and putting it in
a subprocess would confine the wrong thing.

Quarantined content reaches a file through `write_file`'s `contents_ref`.
`Policy::declassify_into_workspace` is what lets a private slot become a file body, and it is
sound only because the destination is inside the boundary the bytes came from: nothing leaves.
Never reach for it for a network body, a command line, or a message to someone.

## Where trusted data comes from

Model output is a function of the model's context and nothing else. So when the context holds
only trusted input, what the model produces is derived only from trusted input, and
`Policy::label_model_output` labels it accordingly. `Policy::context_integrity` tracks this and
only ever falls: one untrusted observation and everything afterwards is untrusted.

This is **not** an upgrade path. It is the first label such text ever receives, assigned from
provenance the kernel tracked. If you find yourself relabelling a value that already has a
label, stop: see the section below.

## Labels only ever degrade

Integrity may go trusted → untrusted. It may **never** go the other way. `Label::degrades_to`
is the check; `Labelled::relabel` returns `None` rather than upgrading.

Never construct a `Labelled` by hand to give a value a better label than its inputs had. That
is laundering, whichever crate it happens in and however well-audited the event looks. If a
value derived from untrusted input needs to be trusted for something to work, the design is
wrong, not the label.

## Routing vs content

Every effect splits into a **routing** part that decides where it lands and a **content**
part that is merely carried:

- Routing must be `(T,pub)`. Untrusted routing is an injection attempt.
- Content may be untrusted, but must not be private at release time.

Reads are the one relaxation: `Policy::promote_confined_read` lets the model choose which
file to read next, because a read cannot change anything and is confined to the workspace. It
must never be used for an effect. Effects need `before_granted_action` and a human
endorsement.

## Layering

- `bua-core` is the kernel. No I/O, nothing prints. Owns the lattice, the gates, and every
  decision derived from content.
- `bua-agent` holds the tools and the turn loop. Carries labelled values; must not inspect them.
- `bua-tui` and `bua-cli` are presentation. May display released content.
- `bua-net` is the single egress chokepoint. All network traffic passes the policy gate here.
- `bua-mcp`, `bua-sandbox`, `bua-signing`, `bua-config` cover extension, confinement, and auth.

Primitives stay native rather than moving behind MCP when the kernel needs to label parts of
a call separately, such as a path as routing and its contents as content. An opaque MCP call erases
that distinction.

## Trust map

`TrustStore` records which paths the user vouched for, keyed by path prefix, longest match
wins. Both polarities are expressible, so trusted-inside-untrusted works as well as the
reverse. Empty means nothing is trusted: trust is granted, never assumed from silence.

A prompt asks one thing: may this path stop being trusted? So the only case that asks is
untrusted data into a trusted path, plus the first write to a path nobody has mentioned, which
is why `integrity_of` returns an `Option`. Writing trusted data never asks, since trusted data
contains nothing an attacker influenced and the destination only gains trust. See the table in
docs/trust.md, which is the specification.

`Policy::reconcile_after_write` keeps the invariant that a path's recorded trust equals the
integrity of the data in it. Untrusted data landing in a trusted tree *must* mark that path
untrusted, or reading it back would launder it. Always the exact path, never the parent.

That invariant does not end with the process. `bua-tui`'s `trust_file` writes the map to
`~/.bua/trust`, one file per working directory, because a path marked untrusted on Tuesday and
read back as trusted on Wednesday is the same laundering with a restart in place of a round trip.
The startup answer applies **on top of** the stored map rather than replacing it, so a fresh yes
cannot un-say a rule a write recorded. A map that will not parse must never send the user back to
the question: nothing is trusted for that session instead, since the rules that would have
overridden the answer are the ones that were lost.

## Absent by design

A **shell** is excluded: a shell string is destination and payload at once, so there is no
separable routing field a person could endorse, and a parser that tried to recover one would be
racing a shell it does not control. `apply_patch` is excluded for the same reason. Before adding a
tool, ask what its routing field is and whether a human could approve that field alone.

Running a **program** is not excluded, because it passes that test. `run` takes a list of argv
stages, never a command string, so an argument containing a metacharacter is one argument and stays
one: there is no parser to defeat and what a person approves is what executes. This is the
distinction to hold onto. It is not that command execution turned out to be acceptable after all,
it is that the exclusion was about shell strings and an argv vector is not one.

The rules `run` lives under, none of which may be relaxed to make something work:

- argv is routing, so it must be `(T,pub)` and endorsed by a person before anything runs. Untrusted
  text never becomes an argument.
- stdin is content, so it may be untrusted. This is what lets untrusted data reach a command line
  without the planner or the driver reading it.
- stdout and stderr are **always** `(U,priv)`. Every stage, no exceptions, nothing a caller or the
  model can declare changes it. This is the property the whole tool rests on.
- Private input asks, even though the labels would permit it, because handing the user's data to a
  program releases it somewhere this policy stops governing.
- Every run asks. There is no read-only category: nothing here can tell whether `foo --bar` writes,
  and a stage declaring itself harmless only helps if the declaration is honest. Do not add one to
  reduce prompting; remembered argv patterns are the direction, and an unprompted write is worse
  than an unwanted prompt.

Programs are **not** confined and **not** enumerated. They run with the access the user's shell
would give them, because `git push` needs `~/.ssh` and the set of programs someone might ask for
cannot be listed in advance. Do not add an allowlist and treat it as the safety property: what holds
is the label on the output, not a belief about the binary. Whether to confine children is issue #4;
whether output can ever be trusted is issue #3. Neither may be resolved by weakening the labels.

## Conventions

- **Never use an em-dash.** Not in documentation, commit messages, the README, code comments,
  pull requests, or anywhere else. Reword instead: a comma, a colon, a semicolon, parentheses,
  or two sentences will always do the job.
- Comments explain **why**, never what. Prefer no comment to a restatement of the code.
- Tests are behavioural and named as sentences. A doc comment on a test says why the property
  matters, not what the test does.
- Test refusals and denials, not just happy paths. A test that would pass against the buggy
  code is worthless: verify a new test fails before the fix.
- Small commits, one property each. Run `make check` (fmt, clippy `-D warnings`, tests)
  before committing.
- No new dependencies without a reason that survives scrutiny. Patterns that arrive through a
  turn are attack surface: prefer literal matching and hand-written, non-backtracking
  matchers to a regex engine.
