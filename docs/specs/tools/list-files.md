---
id: LIST
title: list_files
status: normative
governs:
  - crates/agent/src/glob.rs
  - crates/agent/src/workspace.rs
---

## Scope

Listing what is in a directory. `directory` and `pattern` are routing; there are no content
arguments. The result is the paths, or one reference per entry when the planner may not see them.

## Clauses

<a id="LIST-1"></a>
### LIST-1: a filename is content, so an untrusted listing is quarantined

A listing is trusted only if every file it touched is. A file can be named to read like an
instruction, so the names are treated as content and not shown to the planner.

`verified-by: bravebot_agent::workspace::list_enumerates_files_recursively`
`verified-by: bravebot_agent::turn::untrusted_listings_never_reach_the_model`

<a id="LIST-2"></a>
### LIST-2: a quarantined listing returns one reference per entry, not one for the listing

The planner passes the reference where it would have typed a path, and is never told a filename.

**Why.** One reference for the whole listing would leave the planner holding an address it cannot
use. What came of that in practice was a planner guessing globs to see which came back empty.

`verified-by: bravebot_core::policy::an_entry_reference_names_its_directory_and_never_its_file`
`verified-by: bravebot_core::policy::reserving_the_wrong_number_of_names_is_refused`

<a id="LIST-3"></a>
### LIST-3: the glob is literal and the matcher does not backtrack

The matcher is hand written. `*` and `?` do not cross `/`, `**` does, and brace groups are
unsupported. Version-control and build directories are skipped.

**Why.** A backtracking pattern arriving through a turn is a denial-of-service vector.

`verified-by: none`

<a id="LIST-4"></a>
### LIST-4: a truncated listing says it was truncated

Output is capped, and the cap is reported.

**Why.** Silence would let the planner conclude a file does not exist when the answer was cut off.

`verified-by: none`
