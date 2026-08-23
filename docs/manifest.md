# Manifest mode

`bua "<task>" --mode manifest` plans the whole run before it reads anything, then executes the
plan with no model in the control path. The default, `--mode turn`, is the loop everything else
in this repository describes: read, decide, act, repeat.

This follows the plan-then-execute architecture in the SafeHouse specification, whose three
planning phases and tool schema survive here, with the first phase split into two calls for the
reason given below. SafeHouse is named in
[credit.md](credit.md) as the research proof of concept behind this repository.

## What actually differs

Not the guarantee, and not how strictly it is enforced. Both modes run under the same kernel,
the same gates, and the same rule: untrusted content can be carried and written, and can never
decide what happens. Both precommit routing before anything is read.

What differs is the **scope of the commitment**. A turn precommits routing and the release plan
for one turn and then lets the planner choose the next step from what it has seen. A manifest
run precommits both for the whole run, from a plan fixed while the task string was the only
input in existence.

So the thing manifest mode buys is narrow and worth stating exactly. In turn mode an injected
file cannot redirect an effect, but it can influence which file the planner reads next, because
`Policy::promote_confined_read` is a deliberate relaxation and iteration is useless without it.
In manifest mode it cannot do even that: the set of reads was fixed before the first one
happened.

What it costs is everything a plan cannot know in advance.

## The three phases

**Phase 1, abstract planning.** Two model calls, neither with tools.

*Shape.* What has to happen, in plain words, as a short numbered list. No JSON, no slots, no
capability names, nothing about how any of it will be carried out. This call sees the task and
nothing else.

*Fit.* The same work, expressed for a machine that cannot look at anything before deciding. This
call gets the shape, the capability catalogue, and the rules the manifest is checked against. It
never learns which tool a capability resolves to. If the work cannot be expressed that way it
says so, in its own words, and the run ends there.

The split is where the hard part of this mode becomes visible. A single call tends to produce a
manifest that quietly assumes it will see a result: "read the file, then fix the bug". Fitting is
the step where that has to become read, transform, act, or be declared impossible, and giving it
its own call is what makes a model do it rather than paper over it.

It is also what makes a bad run debuggable, which is the subject of the next section.

Two calls, and still no re-plan. A manifest that fails validation fails the run rather than being
sent back to be fixed.

## Inspecting a run, especially one that failed

`manifest::Attempt` holds everything a run produced, filled as each piece comes into existence
rather than at the end:

| Field | What it answers |
|---|---|
| `shape` | Did the model understand the goal? |
| `proposed` | What did it actually say when asked for a manifest, **verbatim**? |
| `plan` | What did the frozen program turn out to be? |
| `steps` | What did each step do, including the one that failed? |

It comes back on success in `Outcome::attempt` and **on failure in `TurnError::Manifest`**. The
failure path is the one that matters. A run that finished can be judged by its result; a run that
stopped cannot be judged by anything else, and the case that motivates keeping `proposed` raw is
precisely the one where nothing else exists: a manifest that would not parse has no rendered plan,
so the model's own words are all there is to look at.

The CLI prints the report on failure whether or not `--trace` was given, because a one-line
complaint about a document nobody can see is not a failure report. On success it prints under
`--trace`, alongside the audit trail.

Between them these separate four things that otherwise look like one bad run:

- **The model misunderstood the task.** `shape` says so, in plain words, before any machinery.
- **It understood the task and could not express it.** `shape` is right and `proposed` is wrong
  or declines. This is the interesting one, and it usually means the task genuinely needs to look
  before deciding, which is what turn mode is for.
- **The plan was well formed and wrong.** `plan` and `steps` show what actually ran.
- **A gate refused.** The audit trail says which and why.

A cancellation is not a failed attempt and is not reported as one. The user stopped it on
purpose, so there is nothing to explain.

**Phase 2, concrete mapping.** Deterministic, no model. Capability names become tool names
through a registry the model has never seen.

**Phase 3, structural validation.** Deterministic, no model, in the kernel.
`Policy::adopt_manifest` refuses a plan whose label is not trusted, then
`bua_core::manifest::validate` refuses one that is not well formed. Any violation fails the run.
A manifest is never half adopted and a plan is never repaired.

Phase 2 is thinner here than in the specification, and the reason is worth stating rather than
papering over. There it injects provider URLs, MCP tool names and API bases from an
operator-controlled registry, so that a malicious domain cannot reach the planner. A coding agent
working in one directory has no providers to inject: the whole of its world is the workspace the
user opened. What survives is the part that still buys something, which is that the planner names
capabilities and never tool names.

The specification's `CAPABILITY_LABEL` has no counterpart, and its absence is an improvement
rather than a gap. A static table from capability to label cannot express a trust map keyed by
path. So the label a read produces stays what it already was here: the kernel's decision at the
moment of observation, from the path and the user's own trust rules.

## The gates the mode rests on, and what they can actually catch

`Policy::before_planning` refuses to ask for a plan from a planner whose context is not trusted.
`Policy::adopt_manifest` refuses a plan that is not trusted. A plan is a program, so every field
in it is a decision, and a plan derived from something an attacker wrote would be an attacker
choosing the steps.

**Neither can fire while the rest of the kernel is correct, and that is the point.** The planner
is never shown untrusted content, in either mode: `Policy::present` quarantines it and hands over
a reference, and `absorb` only ever runs on what was shown, which is only ever trusted. So the
planner's context is always trusted and these refusals are unreachable today. They are here so the
invariant is stated somewhere that executes, and so that the day something upstream changes, a run
stops rather than planning from an attacker's text.

Which is also why the number of planning calls is not the rule. Counting calls would protect
nothing; what protects the plan is that no call is made from a context that has been shown
untrusted content, and the gate says exactly that.

A word about *observed*, since this repository uses it precisely. `Policy::observe` means a
capability produced a read. Context integrity is about what the planner was **shown**. During
planning nothing has been read either, but for a different reason: the planning policy holds no
read capability at all, so it is not that nothing happened to read, it is that nothing could have.

A trusted plan may then be examined freely, which is what validation does. That is the same
permission `Policy::read_trusted_content` grants and rests on the same fact: the bytes came from
somewhere the user vouched for, so comparing them decides nothing an attacker steers.

## Tiers

| Tier | Capabilities | Model involved | Writes a slot |
|---|---|---|---|
| 1, fetch | `FILE_READ`, `FILE_LIST`, `FILE_SEARCH` | none | yes |
| 2, transform | `TRANSFORM` | an isolated processor: no tools, no memory, one call | yes |
| 3, act | `FILE_WRITE`, `ANSWER` | none | no |

Slots carry data between steps. Everything a step produces is quarantined whatever its label,
because there is no planner left to show it to: the one call that chose the steps finished before
the first step ran. `Policy::quarantine` is that store, and it is deliberately not
`Policy::present`, which exists to decide what a planner may see.

## What the schema enforces

`bua_core::manifest` holds a static contract per tool, the specification's `TOOL_SCHEMA`,
consulted by a pure function. A plan is refused unless:

- every tool is one the schema knows, and every argument is one that tool takes;
- every required field is present and not blank, where `[]` and `""` count as blank;
- every slot a step reads was written by an **earlier** step, and no slot is written twice;
- every path is workspace-relative, with no leading `/` and no `..` component anywhere;
- no action writes a slot, so nothing can depend on what an effect did;
- nothing reads the workspace after the first action;
- at most one step answers the user;
- a write gives exactly one of `contents` or `from_slot`.

A path leaving the workspace is refused here rather than at the filesystem, which already refuses
it. The reason is that the plan is shown to a person before it runs, and `../../.ssh` in a step
someone is being asked to approve is a thing they should never have had to spot.

Several actions may follow one another. The specification requires a driver tool to be the last
step, and that rule is not carried over: with the program frozen, every destination was locked
before the run whatever its position, so position buys nothing extra. What is enforced instead is
the property the rule was protecting, which is that nothing goes back to reading once the plan
has changed something.

## The routing lock and the release plan

Before the first step runs, every destination in the plan is inserted into `Routing` through
`Routing::insert`, which refuses anything not `(T,pub)`. That is the check that holds, not
`adopt_manifest` upstream of it: a plan that somehow arrived untrusted fails at this line even
having passed everything before it. Steps then read their destinations back out of the lock
rather than out of themselves, so the destination in force is the one fixed before execution.

`ANSWER` is the only way anything reaches the user's screen, and the slots it names go into the
`ReleasePlan` at the same moment, before the policy exists. `Policy::declassify` refuses every
other slot. This is the first use in the repository of a release plan that is not empty, and it is
what makes "content cannot nominate itself for release" true rather than vacuous.

Writes are still endorsed by a person, exactly as in turn mode. The plan being frozen says where
a write lands; it does not say that a write should happen without anyone agreeing to it.

## What this mode cannot do

`edit_file` is unavailable: locating a passage to replace means having read the file, and the
planner has read nothing. To change part of a file, read it, transform it, and write the
transform's slot back.

`todo_write` is unavailable because the manifest is a better task list than a narrated one,
having been fixed in advance. The plan is printed before the run for the same reason it exists.

`--file` names files rather than pasting them. In turn mode a named file's contents go into the
planner's context; here the planner is told the paths and plans a `FILE_READ` for each, because
the whole premise is that it decides before it has seen anything.

The attempt is carried out in `Outcome::attempt` and in `TurnError::Manifest`, but it is **not**
written into a session record on disk. `~/.bua/sessions` belongs to the
interactive interface, and manifest mode is a one-shot command with no session to write into. A
caller that wanted to persist them has everything it needs in the outcome; nothing does yet.

There is no conversation and no resuming. The planner is never shown a result, so there is
nothing for a second turn to continue, which is why the mode is a one-shot command and not
something a session can be switched into.

A plan that fails validation fails the run. It is not sent back to the planner to be fixed, so
that the "one call, no round for a reply to steer" property holds at planning time too. The
refusal says which step failed and why, and a person can rephrase.
