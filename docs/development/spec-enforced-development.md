# Spec-enforced development

This project is developed against the mini-specs in [specs/](../specs/), which are the source of
truth for how it behaves: each clause carries the tests that pin it, and is reviewed
closely by a human before it changes. Code under a spec's `governs` list is reviewed against that
spec rather than on its own, and automation checks that every clause still has coverage.

**A clause ships with the work it describes.** One commit carries the behaviour, the tests, and the
clause that specifies it, in one pull request. Do not land the spec separately from the code: a
clause names the tests that pin it, so a spec commit on its own leaves `make check-spec` pointing at
tests that do not exist yet, and a code commit on its own is behaviour nothing specifies. Reviewing
them together is also the only way to see whether the clause and the code agree.

The exception is a spec written to be argued about before anything is built. That is a design
document, it says `verified-by: none` until the work lands, and the commit should say plainly that
it specifies work not yet done. If you are documenting behaviour that already exists, it is not this
case.

Adding a clause means bumping that spec's clause count in [specs/README.md](../specs/README.md);
`make check-spec` fails on a mismatch. Where one commit adds clauses to two specs, that table is
edited by both, so stage it a hunk at a time rather than landing an unrelated count with the wrong
change.
