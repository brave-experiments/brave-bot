# Development

Requires a recent stable Rust toolchain.

```sh
cargo build
cargo test
make check     # fmt, clippy -D warnings, tests, and toolchain age: what CI enforces
```

| Read | For |
|---|---|
| [checks.md](checks.md) | what to run and when, why local clippy is not CI's, and a failure that is not yours |
| [commits.md](commits.md) | what one commit contains, what its message says, and what closes an issue |
| [reviewing-for-the-rule.md](reviewing-for-the-rule.md) | reading a diff against the guarantee the repository exists for |
| [spec-enforced-development.md](spec-enforced-development.md) | developing against the mini-specs, and what ships with a clause |
| [security-scan.md](security-scan.md) | the scan that comments on pull requests, run before pushing |
| [configuration.md](configuration.md) | direnv, what the build captures, and what a built binary holds |
| [agent-configuration.md](agent-configuration.md) | `agents/` as the one source, and the links `make init` creates |
| [localization.md](localization.md) | the catalogs, adding a message or a language, and who a string is for |
| [testing-the-interface.md](testing-the-interface.md) | driving a real terminal for the wiring `cargo test` cannot reach |
| [labelling-issues.md](labelling-issues.md) | the kind label and the three axes every open issue carries |
| [version-stamp.md](version-stamp.md) | which build produced a session, and reading a transcript against it |
| [releasing.md](releasing.md) | cross-builds, naming a version, and the two manual publishes |

[../best_practices.md](../best_practices.md) is what a pull request is reviewed
against: the rules nothing mechanical can decide.
