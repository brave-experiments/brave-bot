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
bua import-leo-creds                 # use a Leo Premium subscription
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

**It can work in files it is not allowed to read.** In an untrusted directory the model never
sees a line of your code. It can still change it: the file goes to an isolated processor, a
second model with no tools, no memory and nothing but that one file, which returns the new
version into quarantine. You see the diff and approve it. Nothing that read the file was in a
position to act, and nothing that acted had read it.

Worth being plain about the trade: a processor is a model call, so the contents of an untrusted
file do reach the backend when you ask for work in one. What changes is not where the bytes go,
since a trusted directory has always sent them there, but that the thing reading them can do
nothing at all.

**It runs programs, but there is no shell.** You can ask for `git commit`, `gh api`, `sed`, `awk`,
or anything else installed, and stages compose the way a pipeline does. What you cannot get is a
shell: `run` takes a program and a list of arguments, never a command string, so there are no pipes,
no `&&`, and no `$(...)`. That is what makes it approvable. A command string is its own destination
and payload at once, with nothing separable for you to see, while an argument list is something you
can read and have executed verbatim. Nothing is escaped or re-parsed, so `; rm -rf /` inside an
argument is just an argument.

**You approve every command.** There is no list of allowed programs and no sandbox around them: they
run with the access your own shell would give them, because `git push` needs your SSH keys and the
set of tools you might ask for cannot be listed in advance. What protects you is that you see the
exact argument list first and your approval covers only that one, so it cannot be reused for a
different command.

**Command output is never trusted.** Whatever a program prints is treated as untrusted and private,
always, since it could contain anything a file or a website put there. Untrusted data can still be
piped *into* a tool, which is what lets `sed` and `awk` work on files nobody vouched for. But the
model receives a description of the output rather than the text, so it cannot read what it just ran.
That is a real limitation and a deliberate one.
[Why this matters](docs/design.md#why-some-things-are-absent), and
[the full model](docs/tools.md#running-programs).

## Trusted directories

At startup you are asked whether you trust the working directory.

**Trust it** and files there are read normally, so ordinary work proceeds without a prompt for
every edit. **Decline** and nothing is trusted, so every write is shown to you first.

Content from an untrusted source, a web page or a file outside a trusted path, is quarantined:
the model can pass it along, hand it to an isolated processor, and write the result somewhere,
but never read any of it, and none of it can decide what happens next. If such content is
written into a trusted directory, that one file is recorded as untrusted, so reading it back
does not launder it.

You are only ever prompted about one thing: **may this path stop being trusted?** The full
rules are in [docs/trust.md](docs/trust.md).

## Configuration

Configuration is built into the released binary, so there is nothing to set up. `bua doctor`
reports what it will use. To point it at a different backend, see
[docs/development.md](docs/development.md#configuration).

## Leo Premium

If you subscribe to Leo Premium in Brave, you can use it here:

```
bua import-leo-creds            # from Brave (stable)
bua import-leo-creds nightly    # or beta, or development
```

Premium requests are then used automatically, and `bua doctor` reports how much is left.
`--forget` discards what was imported.

This **registers as an additional device** rather than borrowing the browser's credentials. Only
the subscription's order id is read from the browser; the credentials themselves are generated
here and signed by Brave, exactly as a second browser on another machine would. The browser keeps
its own, and nothing it holds is spent.

The credentials are single-use and arrive in batches covering a few days. When a batch runs out
or expires, a replacement is obtained automatically, so this is normally a one-time step. They
are kept in the system keychain, not in a file, so importing and the first request of a session
may ask for your password.

Requirements and limits:

- **macOS and Linux.** Windows is not supported.
- The build must know the premium host. Without it premium is unavailable, and a credential is
  never sent to the non-premium host, since it does not belong there.
- A credential only works against the deployment that issued it, so import from the Brave channel
  matching the environment the binary is configured for. Mismatching them returns 401.
- Sign in to Leo in that Brave install first: a subscription that is not in the profile cannot be
  imported.

## Other links

- [How it works](docs/design.md), the labelling model and the six rules it enforces
- [Tools](docs/tools.md), what each tool touches and what it may carry
- [Trusted directories](docs/trust.md), the trust map specification
- [Development](docs/development.md), building, configuring, and the conventions here
- [Credit](docs/credit.md)

## License

[MPL-2.0](LICENSE)
