# brave-user-agent

A coding agent whose defining property is **structural resistance to indirect prompt
injection**.

Point it at a repository and ask it questions. What makes it different is what happens when a
file, a dependency, or a web page it reads contains text designed to hijack the agent: that
text cannot redirect anything, because it never reaches the parts of the system that decide.
The protection is not a filter that has to recognise an attack, so there is nothing for an
attacker to phrase their way past.

> Status: early but working. It answers questions about a real workspace, choosing and
> chaining its own tools, with every decision recorded.

## Install

```sh
npm install -g @brave/user-agent
```

This downloads the release binary for your platform and verifies its checksum. macOS, Linux,
and Windows on both x86_64 and arm64 are supported. To build from source instead, see
[docs/development.md](docs/development.md).

## Using it

```sh
bua                                  # interactive session
bua "what does src/main.rs do?"      # one-shot
bua "explain this" --file notes.md   # with named context
bua doctor                           # check configuration and confinement
```

In a session: the mouse wheel or Up/Down scrolls, Home/End jumps to either end, Ctrl-T
toggles the audit trail, Esc cancels a running turn. Add `--trace` to a one-shot run for the
same audit trail: which gate checked what, the label every value carried, and what was
released.

## What it will and will not do for you

**It reads freely.** The agent picks which files to open as it works, because a read cannot
change anything and it is confined to your working directory.

**It never writes without you.** Every write is your decision, not the model's. Your approval
covers that one exact path and cannot be reused for another. In a one-shot `bua "..."` run
there is nobody to ask, so writes are refused rather than applied unseen.

**Edits are shown as a diff.** An edit names the exact passage to replace, so you review a few
lines rather than a whole file. If that passage is missing or ambiguous, or the file changed
since it was read, the edit is refused instead of guessed.

**There is no shell.** Command execution is absent deliberately, not yet-to-come: a command
string is its own destination and payload at once, so there is nothing separable for you to
approve. [Why this matters](docs/design.md#why-some-things-are-absent).

## Trusted directories

At startup you are asked whether you trust the working directory.

**Trust it** and files there are read normally, so ordinary work proceeds without a prompt for
every edit. **Decline** and nothing is trusted, so every write is shown to you first.

Content from an untrusted source, a web page or a file outside a trusted path, is quarantined:
the model can pass it along and write it somewhere, but never read it, and it can never decide
what happens next. If such content is written into a trusted directory, that one file is
recorded as untrusted, so reading it back does not launder it.

You are only ever prompted about one thing: **may this path stop being trusted?** The full
rules are in [docs/trust.md](docs/trust.md).

## Configuration

Configuration is built into the released binary, so there is nothing to set up. `bua doctor`
reports what it will use. To point it at a different backend, see
[docs/development.md](docs/development.md#configuration).

## Other links

- [How it works](docs/design.md), the labelling model and the six rules it enforces
- [Tools](docs/tools.md), what each tool touches and what it may carry
- [Trusted directories](docs/trust.md), the trust map specification
- [Development](docs/development.md), building, configuring, and the conventions here
- [Credit](docs/credit.md)

## License

[MPL-2.0](LICENSE)
