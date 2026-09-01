---
id: LOCALE
title: Saying it in the reader's language
status: normative
governs:
  - crates/i18n/build.rs
  - crates/i18n/src/*.rs
  - crates/i18n/locales/*.ftl
  - crates/agent/tests/audience.rs
---

## Scope

Every word this program says to a person, where those words live, and which of them may change
with the reader's language.

There are two audiences and only one of them is a reader. A person watching the terminal is one;
the planner is the other, and what it is sent is an interface rather than prose. This spec is
about the first. It says nothing about what the planner is sent beyond forbidding a translation
from reaching it.

## Clauses

<a id="LOCALE-1"></a>
### LOCALE-1: only what a person reads is translated

A catalog holds the interface: the prompts, the confirmations, the status lines, the errors a
person is shown. It does not hold a tool's description, the preamble, the instruction a processor
is given, or the sentence a refused tool answers with. Those go to the planner, and the words in
them are load-bearing on what it does.

**Why.** Translating them would make the agent behave differently in French, which is a change
nobody would find by reading the French. The words the planner reads are pinned by the specs that
govern each tool, and a catalog is not one of them. The line is the audience, not the crate: a
string in `bravebot-agent` that reaches the screen is a message, and a string in `bravebot-tui`
that reaches a request is not.

`verified-by: bravebot_agent::audience::what_the_planner_reads_is_not_taken_from_a_catalog`

<a id="LOCALE-2"></a>
### LOCALE-2: a message is chosen by a name written in the source, never by a value

There is no way to write a lookup whose result depends on something computed while running, so
there is no path by which content the agent was handed can decide what is said to the person
watching. A name no catalog defines does not fall back and does not put a key on the screen: it
fails to build, at the line that wrote it.

**Why.** A message chosen by a value is a branch, and untrusted bytes do not get to take one.
This is the same rule the rest of the system runs on, applied to the one remaining surface that
would otherwise look harmless: the driver deciding which of its own sentences to print is still
the driver deciding.

`verified-by: by-construction (the lookup has one arm per message and no arm accepting an expression, so a key computed at run time does not compile)`

<a id="LOCALE-3"></a>
### LOCALE-3: the reference catalog owns the message set and every argument

`en-US` is the reference. A translation may say each message differently and may leave one out,
falling back. It may not add a message the reference does not have, read an argument the reference
does not pass, or put an argument in a plural the reference states in the singular.

**Why.** Argument names and kinds are what the call sites compile against. A translation that
could change them could break the build of a program nobody had touched, in a language the person
who broke it does not read.

`verified-by: by-construction (a catalog that adds a message, reads an unknown argument, or introduces a select fails the build that would have shipped it)`

<a id="LOCALE-4"></a>
### LOCALE-4: a locale request widens to the language, then to the reference

The catalog named for exactly the locale asked for, else any catalog in the same language, else
the reference. What a shell appends to say which encoding and which modifier is not part of the
name, and the POSIX `C` locale asks for no catalog at all.

`verified-by: bravebot_i18n::lib::a_request_takes_the_catalog_of_that_exact_name`
`verified-by: bravebot_i18n::lib::a_request_with_no_catalog_of_its_own_takes_one_in_its_language`
`verified-by: bravebot_i18n::lib::a_bare_language_takes_a_catalog_in_that_language`
`verified-by: bravebot_i18n::lib::a_language_that_did_not_ship_falls_back`
`verified-by: bravebot_i18n::lib::the_encoding_and_modifier_a_shell_appends_are_not_part_of_the_tag`
`verified-by: bravebot_i18n::lib::the_posix_locale_asks_for_no_catalog`

<a id="LOCALE-5"></a>
### LOCALE-5: a plural is formed by the rules of the language it is read in

A message that counts states its own forms, and which one is used is decided by the rules of the
language the catalog is written in. A language whose rules are not written down cannot ship a
message that counts.

**Why.** Borrowing English's rule is how a translation comes out reading correctly for one and
wrongly for everything else, which is a defect only a speaker of that language can see and which
no review in English will catch.

`verified-by: bravebot_i18n::lib::a_plural_select_picks_the_variant_the_language_calls_for`
`verified-by: bravebot_i18n::lib::a_language_with_a_plural_rule_is_the_only_kind_a_select_may_ship_in`

<a id="LOCALE-6"></a>
### LOCALE-6: no catalog is read while the agent is running

Catalogs are turned into code before the binary exists. A running process holds finished text and
nothing that parses, matches, or interpolates a catalog.

**Why.** A message format is a small language, and an interpreter for one, running in the process
whose whole premise is that it does not interpret what it was given, is the sort of thing that is
only ever noticed afterwards.

`verified-by: by-construction (the build script writes Rust, and the crate links no parser: what ships are string constants and the code that concatenates them)`

<a id="LOCALE-7"></a>
### LOCALE-7: the locale is chosen once, by the program, and not by the library

Nothing about the environment reaches a message until a binary says so, and it says so once,
before anything is drawn. Anything that did not ask reads the reference.

**Why.** A library that consulted the environment the first time something needed a word would
make the output of every test depend on the machine it ran on, and a test that pins what a person
sees is worth nothing if two people running it see different things.

`verified-by: bravebot_i18n::lib::a_process_that_never_chose_a_locale_reads_the_reference`

## Known costs

- **Numbers are not fully formatted for the locale.** A catalog says what separates a whole
  number from its fraction, which covers the two figures in the interface that have one. Digit
  grouping, alternative numerals, and percent and currency forms are not done: each needs the
  CLDR tables, and a partial imitation of them reads worse than a plain number because it is
  wrong only sometimes.

- **The words on the working indicator are not in a catalog.** They are chosen for tone and
  variety rather than for meaning, and translating one word for word keeps neither. A language
  supplies its own list, of its own length, in the interface rather than in a catalog, and one
  that has not supplied any is shown English.

- **The audit trail is not translated.** `--trace` and the trail the interface shows are a
  record of what the system decided, in fixed columns, holding gate and capability names that
  are identifiers rather than words. It is read the way a log is read, and by somebody comparing
  it against the specs that use those same names, so it stays in one language. See
  [trace.md](trace.md).

- **A missing translation is invisible from the English side.** A catalog with fewer messages than
  the reference builds, and the messages it does not have are shown in English. The build says how
  many are missing and nothing fails, because a translation that had to be finished before it
  could be used would never be started.
