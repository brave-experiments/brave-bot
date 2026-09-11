# Checks

## Before a commit

**fmt and clippy.** Both take seconds and have no exemption for a change that only touched a
comment, a document, or a name: they fail on those as readily as on anything else, and `make init`
installs a pre-commit hook that refuses a commit failing either.

Then run the tests the diff is under, scoped as tightly as the diff is: `cargo test -p bravebot-tui
--lib` for the interface, `cargo test -p bravebot-agent --test workspace` for one test binary. A
commit still has to be a state that stands up, and a change nothing covers is a change to be
suspicious of.

## Before pushing

`make check` runs the whole suite and takes minutes. It is what to run before pushing a branch and
what CI runs, not what to run between two edits to the same file, and never twice to confirm the
same thing. Reaching for it out of caution is not free: it is the difference between a review that
takes a minute and one that takes twenty, and the reviewer is the person waiting.

`make check-spec` checks the mechanical half of the specs: clause numbering, the tests each clause
names, the paths it governs, and the table in [../specs/README.md](../specs/README.md).
`make check-npm` installs from the lockfile and lints it, as CI does. `make check-reviewdog` is the
[security scan](security-scan.md).

`make check-linux` runs fmt, clippy and the tests on Linux under the current stable toolchain.
Worth doing before pushing platform-specific code, since a macOS host never compiles the Linux
backend. Its one gap is the Landlock tests: the kernel in play is Docker's, and Docker Desktop's
implements no Landlock at all, so that target sets the switch that skips them instead of failing and
they say so as they go. On a Linux host `make check` runs them against the host kernel already. `make check-msrv` builds against the declared minimum toolchain, which the pinned
cross-build container ships.

## Clippy here is not the clippy CI runs

CI installs whatever stable is current on the day it runs, and clippy gains lints with every
release, so a host a few releases behind passes a warning CI fails on, and the first report of it
is a red build on code that was checked before it was pushed. `make check-toolchain` measures that
gap from the release date rustc states, and it runs last in `make check`. When it fires, the answer
is `make check-linux`. Another local `cargo clippy` is not: it is the same weaker lint set a second
time.

## Reading a result

Read what a command exits with rather than a filter over it: `make check | grep error` reports
success on a formatting failure, because a fmt diff says nothing matching that pattern and grep
exited happily.

**A test that fails on the parent commit is not yours to fix.** Establish that once, cheaply, and
move on: name the test, say it reproduces without the change, and carry on with the work. Do not
bisect it, do not build a baseline worktree for it, and do not re-run the suite hoping. Some tests
here spawn real processes against a wall clock, so they fail on a loaded machine and pass on the
next run; that is a flake, not a signal, and chasing one costs more than the failure does.

If a check cannot pass for a reason outside the change, say so in the commit message rather than
leaving it to be discovered.
