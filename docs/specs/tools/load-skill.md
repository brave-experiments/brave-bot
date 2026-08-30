---
id: LOAD
title: load_skill
status: normative
governs:
  - crates/agent/src/skills.rs
---

## Scope

Fetching the body of a skill named in the system prompt. `name` is routing; there are no content
arguments. The result is the skill's text. Where skills come from and what they are trusted for is
[skills.md](../skills.md).

## Clauses

<a id="LOAD-1"></a>
### LOAD-1: the name selects from a set fixed before the turn, and never becomes a path

It is promoted the way a read path is, but is more confined than one: the name never becomes a
path component, it only picks from a set the driver enumerated before the turn began. A name
holding a traversal matches nothing, because there is no lookup for it to reach.

`verified-by: bravebot_agent::turn::loading_a_skill_that_does_not_exist_is_refused_rather_than_guessed`

<a id="LOAD-2"></a>
### LOAD-2: a name close to a real one is refused, not guessed at

**Why.** Guessing would load instructions nobody asked for.

`verified-by: none`

<a id="LOAD-3"></a>
### LOAD-3: a body reaches the context only when it is asked for

The system prompt advertises names and descriptions and holds no bodies, which is what keeps a
directory of long skills from crowding out the task.

`verified-by: bravebot_agent::turn::a_skill_body_stays_out_of_the_context_until_it_is_asked_for`
`verified-by: bravebot_agent::skills::what_the_prompt_advertises_holds_no_bodies`
