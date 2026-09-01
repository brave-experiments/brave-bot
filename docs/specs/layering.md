---
id: LAYER
title: Layering
status: normative
governs:
  - crates/*/Cargo.toml
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
| `bravebot-agent` | Task execution: the tools and the turn loop | `core`, `aichat`, `config`, `net`, `skus` | Carries labelled values and must not inspect them. `exec` stays argv-only, and `shell` runs only a line a person typed |
| `bravebot-net` | The network egress path for everything carrying labelled content | `core` | All agent traffic passes the policy gate here. See the known cost below |
| `bravebot-aichat` | Client for the OpenAI-compatible aichat backend | `core`, `config`, `net`, `signing` | Speaks the wire protocol only, and reaches the network through `net` |
| `bravebot-tui` | The interactive terminal interface | `core`, `agent`, `aichat`, `config`, `net`, `sandbox` | Presentation. May display released content, always inside a margin it draws itself. Owns the clipboard and shell mode, both of which are gestures a person made. Owns the terminal itself, so it may read the tty directly to ask the terminal about itself; what comes back describes the terminal and never enters a turn |
| `bravebot-cli` | Command-line entry point | `core`, `agent`, `config`, `net`, `sandbox`, `skus`, `tui` | Presentation. Where nobody can be asked, effects are refused rather than applied unseen |
| `bravebot-mcp` | Model Context Protocol client: the extension boundary for tools | `core`, `net`, `sandbox` | An opaque call erases the routing/content split, so primitives stay native rather than moving behind it |
| `bravebot-sandbox` | OS-level confinement for subprocesses | none | Confines processes running code we did not write. A processor's caller is our own code, so it is not what this confines |
| `bravebot-config` | Environment-derived configuration for the backend | none | The user's own configuration surface, on the same footing as the endpoint and the model |
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

## Known costs

- **`bravebot-net` is not the only crate that opens a socket.** `bravebot-skus` builds its own HTTP
  client and talks to Brave's subscription service directly, without depending on `bravebot-net`. That traffic carries credentials and an order id, never workspace
  content or model output, so no labelled value escapes the gate. LAYER-3 is worded as "all agent
  traffic" for that reason. A second egress that ever carried content would be a violation.
