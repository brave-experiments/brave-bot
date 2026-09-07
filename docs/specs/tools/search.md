---
id: SEARCH
title: search
status: normative
governs:
  - crates/agent/src/glob.rs
  - crates/agent/src/workspace.rs
---

## Scope

Finding strings in the workspace. `pattern`, `directory` and `include` are routing; `case_sensitive`
is a property of the call rather than of anything read. There are no content arguments. The result
is the matching lines, or a reference.

## Clauses

<a id="SEARCH-1"></a>
### SEARCH-1: the pattern is a literal substring, never a regular expression

**Why.** A backtracking pattern arriving through a turn is a denial-of-service vector. The
`include` glob is matched by the same hand-written, non-backtracking matcher for the same reason.

`pattern` may be a list, and a line holding any of them matches. That is alternation, not a
pattern language: every entry is still a literal matched with a substring test, so the guarantee
above is untouched: the work is the sum of the patterns, never a power of the input.

Brace groups in `include` are **expanded before the walk**, not matched during it. Each alternative
is an ordinary pattern applied once per path, so a group costs a multiple of the work rather than a
power of it, and an expansion past the cap falls back to matching the pattern literally.

`verified-by: bravebot_agent::glob::a_brace_group_matches_each_alternative`
`verified-by: bravebot_agent::glob::an_oversized_expansion_falls_back_to_the_literal`
`verified-by: bravebot_agent::glob::a_pathological_pattern_does_not_blow_up`
`verified-by: bravebot_agent::workspace::a_search_takes_more_than_one_pattern`

<a id="SEARCH-2"></a>
### SEARCH-2: a result touching several files is trusted only if every one of them is

Otherwise it is quarantined whole. Unlike a listing, a search returns one reference for the whole
result rather than one per hit, so its hits are not addresses.

`verified-by: none`

<a id="SEARCH-3"></a>
### SEARCH-3: a truncated search tells the planner it is incomplete

A complete one makes no such claim, so the planner can tell the difference between "nothing more"
and "nothing more shown".

Every cap counts: one that stopped at the limit on matches, one that stopped before it had opened
every file, and one that ran out of time are equally partial. The last two are the more dangerous,
because with nothing found there is nothing to look incomplete. The claim reaches the planner
whether or not it may read the result, since a notice written inside a body the planner is never
shown tells it nothing.

The file cap is set for a search rather than for a listing, and far above it. A listing's paths are
the answer and each one is spent on context; a search's paths are never shown, and only matching
lines are, which have a cap of their own. Holding a search to a listing's budget bought no context
back and cost whole subtrees.

Which files a capped search kept must not depend on the order the filesystem handed them over. A
walk sorts each directory and takes its own files before descending, so a partial answer is the
same partial answer on every machine and is the shallow part of the tree rather than a scattering
through it.

`verified-by: bravebot_agent::turn::a_truncated_search_tells_the_model_it_is_incomplete`
`verified-by: bravebot_agent::turn::a_complete_search_makes_no_truncation_claim`
`verified-by: bravebot_agent::turn::a_quarantined_search_still_tells_the_model_it_is_incomplete`
`verified-by: bravebot_agent::workspace::a_search_that_could_not_reach_every_file_says_so`
`verified-by: bravebot_agent::workspace::a_search_that_reached_every_file_makes_no_claim`
`verified-by: bravebot_agent::workspace::a_capped_search_keeps_the_same_files_every_time`
`verified-by: bravebot_agent::workspace::a_capped_search_prefers_a_directorys_own_files`

<a id="SEARCH-4"></a>
### SEARCH-4: a search that found nothing for a pattern written as a regular expression says so

Only where nothing was found, and only for a sequence that could not plausibly have been meant
literally. A search that found its pattern is told nothing, and neither is an ordinary substring
that happens to be absent.

**Why.** Nothing found reads as proof the string is absent. For `drop.*file` it proves something
much narrower, and a planner with no way to tell the two apart writes another one: whole rounds go
on rephrasing a question that was never asked. The same reason a truncated search says it is
truncated, for the answer that looks most like a complete one.

The advice is decided from the pattern, which the planner proposed and the routing gate vouched
for; the result decides only whether there was anything to advise about.

`verified-by: bravebot_agent::turn::an_empty_search_for_a_pattern_written_as_a_regex_says_the_match_is_literal`
`verified-by: bravebot_agent::turn::a_search_that_found_something_is_not_lectured_about_its_pattern`
`verified-by: bravebot_agent::turn::an_empty_search_for_a_plain_substring_is_left_to_speak_for_itself`
`verified-by: bravebot_agent::tools::only_a_sequence_that_could_not_be_meant_literally_reads_as_a_regex`

<a id="SEARCH-5"></a>
### SEARCH-5: an empty result says whether anything was searched

A search whose `include` selected no files reports that, and says so instead of reporting no
matches. One that read files and found nothing reports no matches, as before.

**Why.** The two are opposite facts wearing the same sentence. Files were read and the pattern was
not in them, which is evidence about the tree. Or nothing was read at all, which is evidence about
the query and says nothing whatever about the tree. Rendered identically, a planner cannot tell
them apart, and the failure is not hypothetical: a real turn wrote `**/*.{cc,h,mm}` when brace
groups were unsupported, got "(no matches)", retreated to `**/*.cc`, and answered the question
wrong because the files it needed were the two extensions it had just dropped.

Where the glob also leans on syntax the matcher does not have, the result says which, for the same
reason [SEARCH-4](#SEARCH-4) names a pattern written as a regular expression and against the same
failure. Advice is decided from the glob, which the planner proposed and the routing gate vouched
for; the result decides only whether there was anything to advise about.

`verified-by: bravebot_agent::workspace::a_search_says_when_its_include_selected_no_files`
`verified-by: bravebot_agent::workspace::an_include_may_use_a_brace_group`
`verified-by: bravebot_agent::tools::a_glob_leaning_on_missing_syntax_is_named`
`verified-by: bravebot_agent::tools::a_glob_the_matcher_can_read_is_left_alone`

<a id="SEARCH-6"></a>
### SEARCH-6: case sensitivity is asked for, never inferred

A search matches case exactly unless `case_sensitive` is false. Nothing about the pattern widens
it, and a line is reported as it is written rather than as it was folded to match.

**Why.** A search that quietly widened itself would report matches whose reason the caller cannot
see. The alternative to offering the flag is worse than either: a planner that cannot ask for it
mangles the pattern instead, and a real turn searched for `olicy` to get around a capital `P`. That
finds the word it wanted and every other word ending in those letters, with nothing in the result
to say so.

`verified-by: bravebot_agent::workspace::a_search_can_ignore_case`

<a id="SEARCH-7"></a>
### SEARCH-7: vendored and generated directories are not walked

A fixed list of directory names is skipped: version control, build output, caches, and dependencies
fetched or vendored. This is size hygiene applied to **names**, and nothing is read to decide it.

**Why.** A tree that mirrors its dependencies holds far more of them than of its own code, so a
walk that counts them reaches its cap without reaching the project. A real search for a common word
spent its entire budget inside a Rust crate mirror and reported documentation comments about the
wrong meaning of the word.

The project's own `.gitignore` would generalise better and is deliberately not used. It would
decide what to walk from the contents of a file in the tree being walked, and a tree that can hide
its own files from a search is a tree that can hide them from review. The names on the list are
ones no project uses for its own sources, so skipping them needs nobody's word for it.

`verified-by: bravebot_agent::workspace::a_search_skips_vendored_dependencies`
