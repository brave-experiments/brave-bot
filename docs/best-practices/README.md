# Best practices

What a reviewer holds a pull request against. Every rule here is one a person has to read a diff to
decide.

Nothing here restates a rule a tool already enforces or could: formatting, lints, clause numbering,
the lockfiles, an em-dash, an attribution marker, a `declassify` outside the gates. Those belong in
`make check`, `make check-spec`, `make check-npm` and the security scan, and a rule that can be
written as one of those checks is a bug against this directory rather than an entry in it.

| Read | For |
|---|---|
| [specs.md](specs.md) | prose a clause is allowed to be |
| [tests.md](tests.md) | what a test is named, what it covers, and proving it fails first |
| [writing.md](writing.md) | comments, and what is never written anywhere |
| [dependencies.md](dependencies.md) | what a new crate has to be worth, and why a regex is not free |

The review pass over the rule this repository exists for is
[../development/reviewing-for-the-rule.md](../development/reviewing-for-the-rule.md): the four
shapes a violation takes in a diff, and the argument that mistakes a sound design for one.
[../specs/](../specs/README.md) is the source of truth for behaviour, and a spec wins where a spec
and a rule here disagree.
