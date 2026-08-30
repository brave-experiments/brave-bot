---
id: TOOL
title: The tool surface
status: normative
governs:
  - crates/agent/src/tools.rs
---

## Scope

Which tools exist, and which of each call's arguments are routing and which are content. Each tool
has a spec of its own, linked from the table.

## Clauses

### TOOL-1: every argument is routing or content, and the split is fixed here

Routing decides what a tool touches and must be trusted and public. Content is merely carried and
may be untrusted. No argument is both, and nothing at run time reclassifies one.

| Tool | Routing arguments | Content arguments | Result |
|---|---|---|---|
| [`read_file`](read-file.md) | `path`, `path_ref`, `offset`, `limit` | none | the lines, or a reference |
| [`list_files`](list-files.md) | `directory`, `pattern` | none | the paths, or a reference per entry |
| [`search`](search.md) | `pattern`, `directory`, `include` | none | matching lines, or a reference |
| [`write_file`](write-file.md) | `path`, `path_ref`, `contents_ref` | `contents` | confirmation |
| [`edit_file`](edit-file.md) | `path`, `path_ref`, `replace_all` | `old_text`, `new_text` | confirmation |
| [`spawn_processor`](spawn-processor.md) | `reads`, `about` | `instruction` | a reference |
| [`run`](run.md) | every stage's program and arguments | standard input | a reference |
| [`read_output`](read-output.md) | the reference naming the result | none | the bytes, if a person allows it |
| [`load_skill`](load-skill.md) | `name` | none | the skill's text |
| [`todo_write`](todo-write.md) | none | `todos` | confirmation |
| [`ask_user`](ask-user.md) | `questions` | none | what the user answered |

Reads return content when it is trusted and a reference when it is not. Writes are silent or shown
according to the trust map.

`verified-by: bravebot_core::policy::routing_refuses_untrusted_values`
`verified-by: bravebot_core::policy::routing_refuses_private_values`
`verified-by: bravebot_core::policy::fetched_content_can_be_written_but_cannot_choose_the_path`

### TOOL-2: before adding a tool, ask what its routing field is

If a person could not approve that field alone, the tool does not get built. A shell string is
destination and payload at once, which is why the planner has no shell and why `apply_patch` is
excluded. An argument vector passes the test, which is why running a pipeline of stages does not.

`verified-by: none`

### TOOL-3: an unknown tool is reported to the planner rather than ignored

`verified-by: bravebot_agent::turn::an_unknown_tool_is_reported_to_the_model`
`verified-by: bravebot_agent::turn::a_refused_call_is_reported_as_one`
`verified-by: bravebot_agent::turn::each_tool_call_is_announced_before_it_runs_and_summarised_after`
