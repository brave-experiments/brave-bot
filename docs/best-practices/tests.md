# Tests

<!-- applicability: always -->

What a reviewer checks about a test, none of which `cargo test` can: it reports that a test passes,
never that it is worth having.

---

<a id="TS-001"></a>

## Tests are behavioural and named as sentences

**A test name says the property, not the function under test.** The doc comment on a test says why
the property matters, not what the test does. A reader who knows why the property matters can tell
whether the assertions are the right ones, and a reader who has only a restatement of the body
cannot.

---

<a id="TS-002"></a>

## Test refusals and denials, not just happy paths

**The interesting half of this system is what it refuses.** A change that adds a gate, a prompt, a
policy branch or a label transition is covered when the denied path is asserted, not when the
allowed one is.

---

<a id="TS-003"></a>

## A new test fails before the fix

**A test that would pass against the buggy code is worthless.** Run the new test against the
unfixed code and see it fail, then fix, then see it pass. A test written after the fix and never
run against the bug proves the code compiles.

Say in the pull request that the new test reproduces the bug. It is the one fact about a test a
reviewer cannot get from the diff.
