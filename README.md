# brave-bot

Brave Bot is a general-purpose agent, meant as a drop-in replacement for Claude Code, Codex and
opencode. Its defining property is **structural resistance to indirect prompt injection**.

## Getting started

`npm install -g @brave/bravebot`, then run `bravebot` in a repository. See
[docs/getting-started.md](docs/getting-started.md) for installing, running, and what it asks you.

The documentation site at
[brave-experiments.github.io/brave-bot-docs](https://brave-experiments.github.io/brave-bot-docs/)
covers the same ground for somebody using bravebot rather than working on it, and is kept downstream
of the specs below. Its source is
[brave-bot-docs](https://github.com/brave-experiments/brave-bot-docs).

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

## Development

`cargo build` and `make check`, which runs fmt, clippy and the tests. See
[docs/development.md](docs/development.md) for cross-builds, configuration, and the conventions
here.

For the `.envrc` configuration, message bbondy.

## Credit

[docs/credit.md](docs/credit.md).

## License

[MPL-2.0](LICENSE)
