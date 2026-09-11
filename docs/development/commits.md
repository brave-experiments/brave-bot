# Commits and pull requests

**One change per commit, with its tests in that same commit.** A commit is the unit somebody reads,
reverts, and bisects on, so it has to stand up alone: the change, the tests that pin it, and any
documentation the change makes wrong if it lands without it. Tests that arrive a commit later say
the behaviour went in unverified, and a bisect that lands between the two hits a revision passing
for the wrong reason.

Keep them small. If the message needs an "and" to describe what the commit does, it is usually two
commits. Every commit must leave the tree building and passing, since that is the whole of what
makes a history worth bisecting.

A spec clause ships with the work it describes, in that same commit. [spec-enforced-development.md](spec-enforced-development.md) says what
that means and what it costs to do otherwise.

**Close a GitHub issue from the commit and the pull request that finish it.** Where the change fully
resolves the issue, use GitHub's closing syntax (`Closes #123`, `Fixes #123`) so that merging closes
it. Where the change is only part of what the issue asks for, name it without the keyword (`Part of
#123`): an issue closed while the rest of it is outstanding is worse than one left open.

**No co-attribution markers for Claude Code or other tools**, in a commit message or a pull request
body.

**Never use an em-dash.** Not in documentation, commit messages, the README, code comments, pull
requests, or anywhere else. Reword instead: a comma, a colon, a semicolon, parentheses, or two
sentences will always do the job.
