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
arguments. The result is the paths, or one reference per entry where a rule withholds them.

## Clauses

<a id="LIST-1"></a>
### LIST-1: names come with the place, and only a path marked untrusted withholds one

A listing is a fact about a directory rather than a claim about anything in it, and every path one
can return is somewhere the user opened, since that is the whole of what confinement means. So the
names are shown. What is inside each file is a separate question, asked per file when a turn needs
it, and this answers none of it.

A path somebody deliberately marked untrusted is the exception, and it takes the whole listing with
it: "do not look here" covers the names as well as the contents, and a listing is one value.

**Why.** Treating an unmentioned path as untrusted made a listing trusted only if every file in it
had already been vouched for, so a session in an ordinary project could not learn one filename in
its own working directory and guessed at globs instead.

**Known cost.** A filename is content too, and one written to read like an instruction now reaches
the planner. That is the price of the planner being able to see where it is working, and it is
bounded: a name is short, it arrives among other names, and nothing about it decides where an
effect lands.

`verified-by: bravebot_agent::workspace::list_enumerates_files_recursively`
`verified-by: bravebot_agent::workspace::a_listing_covering_a_path_marked_untrusted_withholds_every_name`
`verified-by: bravebot_agent::turn::untrusted_listings_never_reach_the_model`

<a id="LIST-2"></a>
### LIST-2: a withheld listing returns one reference per entry, not one for the listing

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

Output is capped, and the cap is reported. Reported to the planner whether or not it may read the
names, since a listing handed over as one reference per entry is exactly the case where a notice
inside the body reaches nobody.

**Why.** Silence would let the planner conclude a file does not exist when the answer was cut off.

`verified-by: bravebot_agent::workspace::a_listing_past_the_cap_reports_truncation`
`verified-by: bravebot_agent::workspace::a_listing_within_the_cap_reports_no_truncation`
`verified-by: bravebot_agent::turn::a_quarantined_listing_tells_the_model_it_was_capped`
