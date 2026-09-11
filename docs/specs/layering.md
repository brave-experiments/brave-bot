---
id: LAYER
title: Layering
status: normative
governs:
  - crates/*/Cargo.toml
  - crates/*/src/lib.rs
  - crates/*/src/main.rs
  - crates/*/build.rs
---

## Scope

Which crate is allowed to do what. A change to a crate's dependencies or its reach is a change to
this spec.

## Clauses

<a id="LAYER-1"></a>
### LAYER-1: which crate may do what

| Crate | Purpose | Depends on | Constraint |
|---|---|---|---|
| `bravebot-core` | The information-flow kernel: the label lattice, slots, references, capabilities, and every policy gate | none | No I/O, and nothing prints. Owns every decision derived from content, and the only place a declassification witness can be minted |
| `bravebot-agent` | Task execution: the tools and the turn loop | `core`, `aichat`, `bedrock`, `config`, `i18n`, `lsp`, `net`, `sandbox`, `skus` | Carries labelled values and must not inspect them. `exec` stays argv-only, and `shell` runs only a line a person typed |
| `bravebot-net` | The network egress path for everything carrying labelled content | `core` | All agent traffic passes the policy gate here. See the known cost below |
| `bravebot-aichat` | Client for the OpenAI-compatible aichat backend | `core`, `config`, `net`, `signing` | Speaks the wire protocol only, and reaches the network through `net` |
| `bravebot-bedrock` | Client for models on AWS Bedrock | `core`, `aichat`, `config`, `net`, `signing` | Speaks the wire protocol only, and reaches the network through `net`. Runs the AWS CLI to resolve a credential, which is the one subprocess it starts |
| `bravebot-tui` | The interactive terminal interface | `core`, `agent`, `aichat`, `config`, `i18n`, `net`, `sandbox` | Presentation. May display released content, always inside a margin it draws itself. Owns the clipboard and shell mode, both of which are gestures a person made. Owns the terminal itself, so it may read the tty directly to ask the terminal about itself; what comes back describes the terminal and never enters a turn |
| `bravebot-cli` | Command-line entry point | `core`, `agent`, `config`, `i18n`, `net`, `sandbox`, `skus`, `tui` | Presentation. Where nobody can be asked, effects are refused rather than applied unseen |
| `bravebot-mcp` | Model Context Protocol client: the extension boundary for tools | `core`, `net`, `sandbox` | An opaque call erases the routing/content split, so primitives stay native rather than moving behind it |
| `bravebot-lsp` | Language server client: a read-only question about a symbol | `core` | Speaks the protocol only. Asks a closed set of read-only methods and never a name a caller supplies, so a server's own method list cannot widen what this does. Separates a location from the text at it: the type carrying a location holds no text, which is what [tools/lsp.md](tools/lsp.md) rests on |
| `bravebot-sandbox` | OS-level confinement for subprocesses | none | Confines processes running code we did not write. A processor's caller is our own code, so it is not what this confines |
| `bravebot-config` | Environment-derived configuration for the backend | none | The configuration surface, on the same footing as the endpoint and the model. Reads the settings layers, including one a checkout may carry, and hands on rule text without matching a rule. See [backends.md](backends.md) |
| `bravebot-i18n` | Message catalogs for everything a person reads | none | Presentation text only. Holds nothing the planner is sent, and decides nothing: a message is named in the source, so no value can pick one. See [localization.md](localization.md) |
| `bravebot-signing` | Brave services request signing, hs2019 HMAC-SHA256 over the body digest | none | Auth only. Carries no workspace content |
| `bravebot-skus` | Imports a Leo Premium subscription by registering as a new device | none | Auth only. Carries no workspace content and no model output. See [premium-credentials.md](premium-credentials.md) |

`verified-by: by-construction (bravebot-core declares no dependencies at all)`

<a id="LAYER-2"></a>
### LAYER-2: `bravebot-core` and `bravebot-agent` are both the driver

Relocating a decision from one into the other does not remove it. A branch on untrusted bytes is
a violation wherever it sits.

**Why.** The dependency graph makes `core` look like the safe place to put things, and it is not.
The kernel is where decisions derived from content are *taken*, not where they become allowed.

`verified-by: none`

<a id="LAYER-3"></a>
### LAYER-3: presentation crates display untrusted content on purpose

`bravebot-tui` and `bravebot-cli` show quarantined content to the person watching. A terminal is
not a context, and an agent that will not say which file it is working on has protected nobody.
Everything shown is marked with a margin the renderer draws, and has its control characters
replaced, so the content cannot draw its own.

`verified-by: bravebot_tui::marking::quarantined_content_cannot_paint_its_own_margin`
`verified-by: bravebot_tui::render::quarantined_content_is_shown_and_marked_on_every_line`
`verified-by: bravebot_cli::progress::quarantined_content_is_shown_and_marked_on_every_line`
`verified-by: bravebot_cli::progress::quarantined_content_cannot_paint_its_own_margin`

<a id="LAYER-4"></a>
### LAYER-4: a crate root says what it does about unsafe

Every crate root declares `#![forbid(unsafe_code)]`, or `#![deny(unsafe_code)]` with an
`#[allow(unsafe_code)]` at each site that needs one. A library, a binary and a build script are
each a crate root, and a `cfg(test)` module is part of the crate it sits in, so an `unsafe` block
there is one of the crate's own and is named the same way.

**Why.** Nearly every crate here contains no `unsafe` at all. Undeclared, that is a property
nothing records: it holds by accident, and the first `unsafe` to arrive arrives silently. Declared,
the compiler decides it, and what a reviewer reads is the sites that name themselves rather than
every crate in the workspace. Two crates name sites: `bravebot-sandbox`, whose landlock syscalls
are its reason for existing, and `bravebot-skus`, whose tests point `HOME` at a scratch directory.
Taking `deny` where `forbid` would do is the way the rule is kept in letter and lost in substance,
because `deny` is the one an `allow` added later reopens.

`verified-by: bravebot_cli::unsafe_code::every_crate_root_says_what_it_does_about_unsafe`
`verified-by: bravebot_cli::unsafe_code::a_crate_that_exempts_nothing_forbids_rather_than_denies`
`verified-by: bravebot_cli::unsafe_code::allowing_unsafe_at_a_root_is_not_a_declaration`

## Known costs

- **A crate root says nothing about the test binaries beside it.** A file under `tests/` is its
  own crate that no root attribute reaches, so the `unsafe` in `bravebot-agent`'s and
  `bravebot-tui`'s test helpers sits outside what LAYER-4 decides. Nothing in such a file ships,
  and covering them would take an attribute per file with nothing to keep a new file honest, which
  is the accident LAYER-4 exists to remove.

- **`bravebot-net` is not the only crate that opens a socket.** `bravebot-skus` builds its own HTTP
  client and talks to Brave's subscription service directly, without depending on `bravebot-net`. That traffic carries credentials and an order id, never workspace
  content or model output, so no labelled value escapes the gate. LAYER-3 is worded as "all agent
  traffic" for that reason. A second egress that ever carried content would be a violation.
