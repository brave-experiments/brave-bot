---
id: SEARCH
title: search
status: normative
governs:
  - crates/agent/src/glob.rs
  - crates/agent/src/workspace.rs
---

## Scope

Finding a string in the workspace. `pattern`, `directory` and `include` are routing; there are no
content arguments. The result is the matching lines, or a reference.

## Clauses

<a id="SEARCH-1"></a>
### SEARCH-1: the pattern is a literal substring, never a regular expression

**Why.** A backtracking pattern arriving through a turn is a denial-of-service vector. The
`include` glob is matched by the same hand-written, non-backtracking matcher for the same reason.

`verified-by: none`

<a id="SEARCH-2"></a>
### SEARCH-2: a result touching several files is trusted only if every one of them is

Otherwise it is quarantined whole. Unlike a listing, a search returns one reference for the whole
result rather than one per hit, so its hits are not addresses.

`verified-by: none`

<a id="SEARCH-3"></a>
### SEARCH-3: a truncated search tells the planner it is incomplete

A complete one makes no such claim, so the planner can tell the difference between "nothing more"
and "nothing more shown".

`verified-by: bravebot_agent::turn::a_truncated_search_tells_the_model_it_is_incomplete`
`verified-by: bravebot_agent::turn::a_complete_search_makes_no_truncation_claim`
