# bravebot

Brave Bot is a general-purpose agent, meant as a drop-in replacement for Claude Code, Codex and
OpenCode. Brave has always shipped user agents, and this one is the user's agent in both senses:

- Structural resistance to indirect prompt injection: a web page cannot redirect an agent working
  on your behalf.
- As few tokens as possible.
- As fast as it can be.
- Nothing hidden: what it read and what it decided are on the screen.
- Developed completely in the open.
- No lock-in: other providers and models, settings on your own machine.
- The power of a shell without the risk: command lines are compiled into the trust model rather than handed to one.

## Getting started

```sh
npm install -g @brave/bravebot
```

Or on macOS and Linux, without npm:

```sh
curl -fsSL https://raw.githubusercontent.com/brave-experiments/bravebot/main/install.sh | sh
```

That puts the binary in `/usr/local/bin`, and takes `INSTALL_DIR` for somewhere else:

```sh
curl -fsSL https://raw.githubusercontent.com/brave-experiments/bravebot/main/install.sh | INSTALL_DIR="$HOME/.local/bin" sh
```

Either way, run `bravebot` in a repository afterwards. Both installers verify the release
checksum, and a session started on an old version says so and gives the line that updates it. See
[docs/getting-started.md](docs/getting-started.md) for installing, running, and what it asks you.

The documentation site at
[brave-experiments.github.io/bravebot-docs](https://brave-experiments.github.io/bravebot-docs/)
covers the same ground for somebody using bravebot rather than working on it, and is kept downstream
of the specs below. Its source is
[bravebot-docs](https://github.com/brave-experiments/bravebot-docs).

## How it works

Before data is processed it is labelled as trusted or untrusted and as public or private.
An example of untrusted content is text from a web page. An example of private data is a project's secrets.
Brave Bot can work with untrusted and private content, but it never lets that content into its planner's context.
Processors are used to work on immutable untrusted content. A processor is a sub-agent with no tools, no memory and no conversation. It can read untrusted content and rewrite it, but nothing it produces can direct what happens next.
One or more labelled data slots are passed into a processor, and it outputs at most one new immutable data slot.
The planner is never influenced by untrusted context.

Traces of the gates running are in [docs/specs/](docs/specs/README.md).

## Specs

Behaviour is specified clause by clause, and each clause names the tests that pin it. See
[docs/specs/](docs/specs/README.md).

## Data collection, usage, and retention

We do not use your data and we do not store it. Prompts and used file contents are sent to Brave's
endpoint to produce a reply and are discarded once it has been produced. Nothing is retained and
nothing is used for training. Local settings are stored in `~/.bravebot` on your own machine.

`bravebot --incognito` runs a session that adds nothing to `~/.bravebot`: no prompt history, no
session record, no audit trail, and no change to the model or theme you have chosen. It still reads
all of that, so the session is the one you configured rather than a fresh install. It edits your
project as usual, since that is the work rather than a trace of it. See
[docs/specs/incognito.md](docs/specs/incognito.md) for what it covers and what it does not.

## Development

`cargo build` and `make check`, which runs fmt, clippy and the tests. See
[docs/development/](docs/development/README.md) for checks, commits, cross-builds, configuration
and releasing, and [docs/best-practices/](docs/best-practices/README.md) for what a pull request is
reviewed against.

For the `.envrc` configuration, message bbondy.

## Credit

[docs/credit.md](docs/credit.md).

## License

[MPL-2.0](LICENSE)
