---
id: LIST
title: list_files
status: normative
governs:
  - crates/agent/src/glob.rs
  - crates/agent/src/workspace.rs
---

## Scope

Listing what is in a directory. `directory`, `pattern` and `depth` are routing; there are no
content arguments. The result is the paths, or one reference per entry when the planner may not
see them.

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

Output is capped, and the cap is reported. Reported to the planner whether or not it may read the
names, since a listing handed over as one reference per entry is exactly the case where a notice
inside the body reaches nobody.

**Why.** Silence would let the planner conclude a file does not exist when the answer was cut off.

`verified-by: bravebot_agent::workspace::a_listing_past_the_cap_reports_truncation`
`verified-by: bravebot_agent::workspace::a_listing_within_the_cap_reports_no_truncation`
`verified-by: bravebot_agent::turn::a_quarantined_listing_tells_the_model_it_was_capped`

<a id="LIST-5"></a>
### LIST-5: a listing walks the whole tree unless it is given a depth, and says where it stopped

`depth` is how many directory levels below `directory` are walked. One is that directory and no
further. Absent, the walk reaches every file under it that a permission rule does not cover, which
is what a caller that names no depth gets. A rule keeps a path out of the listing and a directory
out of the walk; see [permissions.md](../permissions.md).

A directory a bounded walk did not descend into is named in the result alongside the files, so
what comes back describes the shape of the tree and not only the part of it that was read. The
pattern does not apply to those: it says which files are wanted, and a directory is where the
answer might be rather than an answer.

**Why.** Without a bound the only listing on offer is every path at every depth, which in a real
repository is thousands of them. That is paid for in the planner's context, again on every round
that resends it, and again in each delegate handed the same question. The common question is what
a project holds rather than every file it contains, and a listing bounded to one level answers it
in the space of a screen.

**Why the directories.** A bounded listing of files alone describes a tree with no branches. A
planner reading one concludes there is no source directory and looks no further, which is worse
than the listing it was trying to avoid.

`verified-by: bravebot_agent::workspace::a_listing_given_a_depth_descends_no_further_than_that`
`verified-by: bravebot_agent::workspace::a_depth_limited_listing_names_the_directories_it_stopped_at`
`verified-by: bravebot_agent::workspace::a_pattern_does_not_hide_the_directories_a_bounded_walk_stopped_at`
`verified-by: bravebot_agent::workspace::a_listing_with_no_depth_walks_the_whole_tree`
