---
id: SKILL
title: Skills and standing instructions
status: normative
governs:
  - crates/agent/src/skills.rs
guards:
  - symbol: Policy::label_user_configuration
  - symbol: Policy::read_trusted_content
---

## Scope

The two kinds of file that steer a turn before the user types anything: `AGENTS.md`, which says
how work is done somewhere, and a skill, which says how one kind of task is done. What a skill
file looks like, what each source is trusted for, and how one is loaded.

Which files are looked for, where, and in what order is a separate question, and it is
[instructions.md](instructions.md) that answers it.

## Clauses

<a id="SKILL-1"></a>
### SKILL-1: a skill is one `SKILL.md` with `name` and `description` in frontmatter

Both keys are required, and a file missing either is skipped with a note saying so. Other keys are
ignored, so a skill written for another agent works here. A file with no frontmatter is not a
skill.

```markdown
---
name: commit-style
description: How commit messages are written here. Use before writing one.
---

Write the subject in the imperative. Explain why in the body, never what.
```

`verified-by: bravebot_agent::skills::frontmatter_without_a_name_or_description_is_skipped`
`verified-by: bravebot_agent::skills::a_file_with_no_frontmatter_is_not_a_skill`
`verified-by: bravebot_agent::skills::an_unterminated_frontmatter_block_is_skipped_rather_than_swallowing_the_body`
`verified-by: bravebot_agent::skills::keys_other_than_name_and_description_are_ignored`
`verified-by: bravebot_agent::skills::the_body_is_everything_after_the_closing_marker`
`verified-by: bravebot_agent::skills::a_marker_inside_the_body_is_left_alone`

<a id="SKILL-2"></a>
### SKILL-2: only the name and description reach the prompt; the body waits for `load_skill`

The description is what the planner decides from, so it should say *when* to use the skill rather
than what it contains.

**Why.** A directory of long skills would otherwise crowd out the task.

`verified-by: bravebot_agent::skills::what_the_prompt_advertises_holds_no_bodies`
`verified-by: bravebot_agent::turn::a_skill_body_stays_out_of_the_context_until_it_is_asked_for`
`verified-by: bravebot_agent::skills::skills_are_offered_in_the_same_order_every_time`

<a id="SKILL-3"></a>
### SKILL-3: `~/.bravebot` is trusted by provenance, never by the trust map

It is labelled from provenance, because the trust map is keyed by workspace-relative paths and has
nothing to say about a path outside the workspace. Never label a workspace path this way: that
would be laundering.

Nothing is assumed from silence. An empty directory offers nothing, and putting a file there is
the grant, on the same footing as the configuration that picks the model and the endpoint.

`verified-by: bravebot_agent::skills::a_home_skill_is_not_labelled_by_a_rule_meant_for_the_workspace`

<a id="SKILL-4"></a>
### SKILL-4: a project's own files are read through the trust map

A workspace `AGENTS.md` and `.bravebot/skills` are labelled as workspace content, so TRUST decides.
`.bravebot/skills` is checked for trust **before it is enumerated at all**, because a directory
name is content too.

`verified-by: bravebot_agent::skills::a_skill_in_an_untrusted_project_is_not_named_to_the_planner`
`verified-by: bravebot_agent::skills::a_skill_the_trust_map_distrusts_stops_being_offered`

<a id="SKILL-5"></a>
### SKILL-5: a source that fails `read_trusted_content` is dropped entirely, never quarantined

Both `~/.bravebot` and a workspace source pass the trusted-content gate on the way into the system
prompt, and a refusal drops the source.

**Why.** A reference to an instruction is no use to anyone: an instruction is either followed or
absent, and one from a directory nobody vouched for has to be absent. A skill's name and
description are content that would otherwise go into the prompt verbatim.

`verified-by: bravebot_agent::skills::an_untrusted_skill_is_not_named_in_what_the_user_is_told`

<a id="SKILL-6"></a>
### SKILL-6: what was skipped is counted, never named

```
AGENTS.md was not loaded: this directory is not trusted
2 skills in .bravebot/skills were not loaded: this directory is not trusted
```

**Why.** A directory in an untrusted project can be named to read like an instruction, and that
name would be on the user's screen as though the agent had written it.

`verified-by: bravebot_agent::skills::a_skill_that_was_skipped_is_counted_rather_than_passed_over_in_silence`

<a id="SKILL-7"></a>
### SKILL-7: withdrawn, the most specific source wins

Replaced by [instructions.md](instructions.md), which owns the order sources are read in along
with the rest of resolution.

<a id="SKILL-8"></a>
### SKILL-8: `load_skill`'s name is never a path

It selects from the set found before the turn started, so a name holding `../` or an absolute path
matches nothing and the call is refused: there is no lookup for it to reach. A name merely close to
a real one is refused too, rather than guessed at, since guessing would load instructions nobody
asked for.

`verified-by: bravebot_agent::turn::loading_a_skill_that_does_not_exist_is_refused_rather_than_guessed`

<a id="SKILL-9"></a>
### SKILL-9: withdrawn, sources are looked for afresh every turn

Replaced by [instructions.md](instructions.md), which owns when resolution runs.

<a id="SKILL-10"></a>
### SKILL-10: a value may be wrapped over the lines indented beneath it

A description says *when* to use a skill, so it runs to a sentence or two and files wrap it. The
lines indented under a key continue that key's value however the file spells the wrap: folded or
literal with `>` or `|`, quoted and carried over, or plain text simply continued. A folded value
is joined with spaces, because the line breaks were the file's and not the sentence's, and a
literal one keeps the newlines it asked for. Quotes around a value are the file's syntax and are
not part of it.

**Why.** A continuation line is not a declaration. Reading one as a key ends the value where the
line ended, which puts half a sentence in the prompt or, where the wrap began immediately after
the colon, an empty value and a skill dropped for being half-declared.

`verified-by: bravebot_agent::skills::a_value_wrapped_over_several_lines_is_one_value`
`verified-by: bravebot_agent::skills::a_continuation_line_holding_a_colon_does_not_start_a_new_key`
`verified-by: bravebot_agent::skills::a_folded_block_becomes_one_line_and_a_literal_block_keeps_its_own`
`verified-by: bravebot_agent::skills::the_quotes_around_a_scalar_are_not_part_of_it`

<a id="SKILL-11"></a>
### SKILL-11: a notice is said when it is learned, not when the turn ends

What loaded and what did not is known before the first request goes out, and that is when it is
said. A turn that fails, is cancelled, or never reaches an answer has said it already.

**Why.** A notice describes what the turn is about to work with. Held until the turn is over it
arrives after every tool line, reading as the last thing that happened rather than the first, and
a turn with no outcome carried none at all: the run where a missing skill mattered most was the
run that said nothing about it.

`verified-by: bravebot_agent::turn::what_did_not_load_reaches_the_interface_when_it_is_learned`
`verified-by: bravebot_agent::turn::what_did_not_load_is_reported_even_when_the_turn_never_finishes`

## Known costs

- **A skill downloaded into `~/.bravebot/skills` is trusted exactly as far as a config file the
  user pasted is.** The name, the description and the body all go to the model as instructions.
  Nothing downstream second-guesses it, because everything downstream is built to trust what the
  user vouched for. Read one before installing it.
