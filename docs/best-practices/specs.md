# Specs

<!-- applicability: always -->

Prose rules for a clause, checked by reading it. Clause ids, front matter, coverage and what a spec
may refer to are in [../specs/README.md](../specs/README.md), and `make check-spec` enforces them.
Read that file before writing or changing a spec, rather than copying the shape of whatever spec is
nearest.

---

<a id="SP-001"></a>

## A spec says what is true now

**Present tense, no history.** A clause describes the behaviour as it stands and the reasoning that
holds it in place, so it reads the same whether it was written today or two years ago.

Keep out of a spec: what an earlier version did, what was tried and abandoned, and which bug
prompted a change. A spec is read by somebody who has never seen any other version of this system,
and what changed is in the commit that changed it.

This binds the commentary and the **Why** of a clause as much as the clause itself: argue from what
the alternative costs, in the present tense, not from what the code did last week.

A measurement is different from a story about one. "Denying writes leaves the index unsettled" is a
fact about the system and belongs in the clause it justifies. "I tried denying writes and it broke"
is the same fact wearing a diary entry, and does not.

---

<a id="SP-002"></a>

## A known cost is a weakness, not a lesson

**A known cost is present tense too:** it names a weakness the design still has, not a mistake
somebody made on the way, and not a limitation that arrived. If the sentence is only interesting
because of who learned it and when, it is not a known cost.

An **open question** is for a decision genuinely unsettled, not for a thing that was settled and is
being justified after the fact.

---

<a id="SP-003"></a>

## A spec is written to be checked, not admired

**A clause is read by somebody deciding whether a diff obeys it,** so every sentence should be one
they could hold a diff against. Cut the cadence, the flourishes and the sentences that only set a
mood: plain declarative statements, and a **Why** that gives the reason rather than performing it.
