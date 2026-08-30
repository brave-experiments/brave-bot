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

Both caps count: one that stopped at the limit on matches and one that stopped before it had
opened every file are equally partial. The second is the more dangerous, because with nothing
found there is nothing to look incomplete. The claim reaches the planner whether or not it may
read the result, since a notice written inside a body the planner is never shown tells it nothing.

`verified-by: bravebot_agent::turn::a_truncated_search_tells_the_model_it_is_incomplete`
`verified-by: bravebot_agent::turn::a_complete_search_makes_no_truncation_claim`
`verified-by: bravebot_agent::turn::a_quarantined_search_still_tells_the_model_it_is_incomplete`
`verified-by: bravebot_agent::workspace::a_search_that_could_not_reach_every_file_says_so`
`verified-by: bravebot_agent::workspace::a_search_that_reached_every_file_makes_no_claim`
