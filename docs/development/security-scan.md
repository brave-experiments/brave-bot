# The security scan

Pull requests here are scanned by an organization-level workflow that runs
[brave/security-action](https://github.com/brave/security-action) and comments through reviewdog.
Nothing in this repository configures it, so the runners and the rule set live over there.

```sh
make check-reviewdog       # what this branch changed, against its merge base
make check-reviewdog-full  # the whole tree, however old the finding is
```

`contrib/check-reviewdog.sh` clones that repository, pins the tool versions its `action.yml` pins,
and drives the same reviewdog runners against this checkout: **opengrep** (the semgrep fork, on
Brave's rule set), **npm-audit**, **pip-audit**, **safesvg**, and **sveltegrep**. Only the runners
with something to look at are enabled, which here means opengrep and npm-audit. A finding is one
the bot would post, so run `check-reviewdog` before pushing; the full scan reports plenty that
predates any given branch.

The first run downloads opengrep, reviewdog and the rules into `~/.cache`; later runs re-use them
and take about half a minute. No model is involved, so both are deterministic.
