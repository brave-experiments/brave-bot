# Catalogs

One file per locale, named for it: `fr.ftl` for a language, `fr-CA.ftl` where a region needs its
own. `en-US.ftl` is the reference and the fallback.

## Adding a language

Copy `en-US.ftl` to a file named for the locale, translate the values, and build. That is the
whole of it: the build script finds the file, compiles it, and the binary offers it.

Nothing else changes. The messages a translation leaves out are shown in English, and the build
says how many those are, so a catalog is useful from its first line rather than from its last.

## What the build will not let through

- **A message the reference does not have.** Messages are added to `en-US.ftl` first, so the call
  sites exist before a translation of them does.
- **An argument the message does not take.** `{ $name }` has to be one the reference passes, and
  the spelling has to match.
- **A plural where the reference has none.** The argument's Rust type comes from the reference, so
  a message that needs a plural form in your language needs one in the reference first. Ask for it.
- **A plural in a language whose rules are not written down.** Those live in `../src/plural.rs`,
  and a language missing from there takes nobody else's rule: it fails the build instead. Add it.

## The format

A subset of Project Fluent, so `.ftl` tooling reads these. What the subset has:

```ftl
# A comment.
write-title = approve this write?

# A long value continues on indented lines, and is one line of text: the wrapping happens
# at the width it is drawn at, so break these wherever the file reads best.
vouch-explained =
    the model cannot read this file, so it is working blind on it. Vouching lets it read
    this file for the rest of this session, here and in every later read.

# An argument is a hole the interface fills.
session-renamed = renamed to { $title }

# A message that counts states its forms. `*` marks the one used when nothing else answers,
# and an exact number wins over the category for that number.
count-rules = { $count ->
    [one] { $count } rule
   *[other] { $count } rules
    }
```

What it does not have: nested selects, terms, functions, and attributes. A select is the whole
value or none of it, so text either side of one goes inside each variant.

## What is deliberately absent

Words the model reads. A tool's description, the preamble, and the sentence a refused tool answers
with are interface to a model rather than prose for a person: translating them would change what
the agent does, in a language whoever changed it does not read. `[Image #2]` is in that group too,
because the planner counts markers to find the picture that answers one.

The words on the working indicator are absent for a different reason. They are chosen for tone and
oddity rather than meaning, so a language writes its own list in `../../tui/src/verbs.rs` instead
of translating these.

[localization.md](../../../docs/specs/localization.md) is the spec.
