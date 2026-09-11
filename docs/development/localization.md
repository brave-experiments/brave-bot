# Words a person reads

Every one of them lives in `crates/i18n/locales/`, one file per locale, and reaches the screen
through `t!(some_message)`. `en-US.ftl` is the reference: it owns the set of messages and the name
and kind of every argument, so a translation can add none of its own and break no call site.

```sh
make locales     # what each translation has of the reference, and what it is missing
```

Adding a language is copying `en-US.ftl` and translating it. No Rust changes, no registration:
the build script finds the file. See
[crates/i18n/locales/README.md](../../crates/i18n/locales/README.md), which is written for whoever
is doing the translating rather than for whoever wrote this.

Adding a **message** means adding it to `en-US.ftl` first, because the macro has one arm per
message in the reference and a name no catalog defines does not compile. The other catalogs can
follow later; what they lack is shown in English.

The distinction that matters here is the audience, not the crate. The words the planner reads are
not in a catalog and must not be: a tool's description, the preamble, and the sentence a refused
tool answers with are interface to a model, and rewording them in another language changes what
the agent does. `crates/agent/tests/audience.rs` fails if a catalog lookup appears in one of those
modules. [specs/localization.md](../specs/localization.md) is the spec, and its known costs list what
is deliberately left in English.
