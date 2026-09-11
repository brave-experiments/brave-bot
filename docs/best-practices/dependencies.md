# Dependencies

<!-- applicability: always -->

A new crate widens the supply-chain surface of a binary people install, and
nothing in CI decides whether that trade is worth making.

---

<a id="DEP-001"></a>

## No new dependency without a reason that survives scrutiny

**A diff that touches `Cargo.toml` or `package.json` says in the pull request
what the dependency buys and what writing it by hand would cost.** Convenience
is not an argument on its own. Depth counts: a crate that pulls in twenty others
is twenty decisions, not one.

---

<a id="DEP-002"></a>

## Prefer literal matching to a regex engine

**Patterns that arrive through a turn are attack surface.** Prefer literal
matching and hand-written, non-backtracking matchers to a regex engine,
particularly anywhere a pattern could come from content rather than from this
repository's own source.
