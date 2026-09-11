# Reviewing for the rule

Everything here is predicated on one statement: **untrusted content never enters the driver's
context or the planner's**. The *planner* is the model deciding what to do next. The *driver* is
this Rust code, meaning `bravebot-core` and `bravebot-agent` both. Both halves are stated as
clauses in [specs/labels.md](../specs/labels.md).

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

**3. A `declassify` outside the gates.** A witness is not permission to inspect. Minting one
records that bytes moved somewhere they were already allowed to go: a filesystem write, an HTTP
body, or a person's screen. Each of those has a gate of its own, `Policy::present`,
`Policy::render_in_place` and `Policy::read_trusted_content`, and the planner's own arguments
have `Policy::read_planner_argument`. A `declassify` anywhere else is almost certainly a
violation, and it can only be written inside the policy layer: `Declassification::authorise` is
`pub(in crate::policy)`, so no other module and no other crate can mint one at all.

**4. A `Labelled` built by hand.** Never construct one to give a value a better label than its
inputs had. That is laundering, whichever crate it happens in. If a value derived from
untrusted input has to be trusted for something to work, the design is wrong, not the label.

Two places in the kernel do branch on untrusted bytes, deliberately. Both are named under Known
costs in [specs/labels.md](../specs/labels.md), because an unlisted exception is indistinguishable
from a violation.

## The inverse mistake: inventing a violation

This rule is about content that could reach the planner or steer a turn in progress. It is not a
general prohibition on reading bytes that arrived over a network, and it says nothing about this
program's own configuration and startup.

`GET /v1/models` is the example to keep in mind. It is fetched before any session, from the endpoint
the user configured, so a person can pick a model off a list they read. It carries
`Label::untrusted_public()` because that label records *where bytes came from*, not that the data is
injection-sensitive, and the code already branches on it freely: `usable` filters on
`capabilities`, compares `access` against `premium`, and `adopt_window` takes a number out of the
same response to decide when a conversation is shortened. None of that is a violation, and an
argument implying it is has misread the rule.

So before reaching for a trust or label argument, ask whether the bytes could reach a model's
context or influence a turn already running. If they could not, the question is an ordinary
engineering one: latency, offline behaviour, staleness, how many code paths. Argue it on those
terms.

**Why this is worth a section.** A wrong trust argument reads exactly like a safety feature, which
is the same thing that makes a real violation hard to spot, and it is more likely to be waved
through than a plain design mistake. It also talks you out of the better implementation for a reason
that does not exist. Getting the scope of the rule wrong in this direction is a real cost, not a
harmless excess of caution.
