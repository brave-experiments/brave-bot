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
wrong — do not restate the rule to match the code.

`Labelled<T>` enforces the mechanical half of this at compile time: no `Deref`, `PartialEq`,
or `Display`, and no infallible accessor. Reading the value requires a `Declassification`
witness only the policy layer can mint. **A `declassify` call is a claim that no decision
follows.** Releasing content to a filesystem write, an HTTP body, or a human's screen is
fine. Using it to choose whether an effect happens is not.

Watch for this in review. The subtle violations look like safety features:

```rust
// WRONG: the driver decided whether to write, from untrusted bytes.
let text = contents.declassify(&proof);
if text.matches(old).count() > 1 {
    return "error: ambiguous";
}
```

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

- `bua-core` — the kernel. No I/O, nothing prints. Owns the lattice, the gates, and every
  decision derived from content.
- `bua-agent` — tools and the turn loop. Carries labelled values; must not inspect them.
- `bua-tui` / `bua-cli` — presentation. May display released content.
- `bua-net` — the single egress chokepoint. All network traffic passes the policy gate here.
- `bua-mcp`, `bua-sandbox`, `bua-signing`, `bua-config` — extension, confinement, auth.

Primitives stay native rather than moving behind MCP when the kernel needs to label parts of
a call separately — a path as routing, its contents as content. An opaque MCP call erases
that distinction.

## Absent by design

Command execution is not missing, it is excluded: a shell string is destination and payload
at once, so there is no separable routing field a person could endorse. `apply_patch` is
excluded for the same reason. Before adding a tool, ask what its routing field is and whether
a human could approve that field alone.

## Conventions

- Comments explain **why**, never what. Prefer no comment to a restatement of the code.
- Tests are behavioural and named as sentences. A doc comment on a test says why the property
  matters, not what the test does.
- Test refusals and denials, not just happy paths. A test that would pass against the buggy
  code is worthless — verify a new test fails before the fix.
- Small commits, one property each. Run `make check` (fmt, clippy `-D warnings`, tests)
  before committing.
- No new dependencies without a reason that survives scrutiny. Patterns that arrive through a
  turn are attack surface: prefer literal matching and hand-written, non-backtracking
  matchers to a regex engine.
