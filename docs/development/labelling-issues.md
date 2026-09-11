# Labelling an issue

The kind labels say what an issue **is**: `bug`, `security`, `spec-mismatch`, `spec-coverage`,
`spec-bug`, `parity`, `enhancement`. The three axes say what to **do** about it, and every open
issue carries one value from each: an `importance`, an `urgency`, and a `size`.

A missing axis is not a low value. It means nobody has judged the issue, and an unjudged issue is
invisible to every query built over the backlog: it is in no importance ordering, no urgency queue,
and no list of what fits in an afternoon.

**Importance is not urgency**, and they come apart in both directions. A hole in the guarantee that
nothing currently reaches is `importance/p1` and `urgency/p3`: it has to be fixed, and the tree is
no worse tomorrow for it. A parsing slip that eats a person's typed line is `importance/p4` and
`urgency/p2`: small, and it costs somebody something every day it stands. Collapsing the two into
one number is what loses the work in the middle.

| `importance` | What leaving it costs |
|---|---|
| `importance/p1` | the guarantee, or a person's data; everything else is downstream of it holding |
| `importance/p2` | central to what the tool is for |
| `importance/p3` | an ordinary defect, or a capability worth having |
| `importance/p4` | a narrow audience, or a small gain |
| `importance/p5` | cosmetic, the tree is no worse for leaving it |

| `urgency` | When it gets done |
|---|---|
| `urgency/p1` | stop other work and fix it |
| `urgency/p2` | next, ahead of planned work |
| `urgency/p3` | scheduled work, take it in turn |
| `urgency/p4` | no deadline, do it when the area is open |
| `urgency/p5` | indefinite, waiting on a decision or on work not done |

`urgency/p1` says every other thing in flight should be put down, so the list is expected to be
empty, and applying one is a claim about everybody's day rather than about the issue.

What earns `urgency/p2` is a cost that grows: reachable now, worsening with time, or taking the
session down under somebody who is using it. An issue that is merely important does not earn it,
because `importance` already records that.

| `size` | What it takes |
|---|---|
| `size/1` | an hour or less: one call site, and the test that pins it |
| `size/2` | a sitting: one subsystem, a handful of call sites |
| `size/3` | a few days: several files, a spec clause, new tests |
| `size/4` | a week or more: crosses crates, or a design to settle first |
| `size/5` | a project: a design document, several commits, a staged landing |

Size counts everything that ships in the commit, the tests and the spec clause included, and not
the lines of the fix alone. It is scheduling information and never a reason to skip something: a
`size/4` at `importance/p1` is work to split, not work to leave.

The three read together. Importance orders the backlog, urgency interrupts that order, and size
says what fits in the time available.

The [triage-issues skill](../../agents/skills/triage-issues/SKILL.md) assigns the triple to every
issue that survives a run, and takes the meanings from here.
