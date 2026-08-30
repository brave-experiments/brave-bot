---
id: TODO
title: todo_write
status: normative
governs:
  - crates/core/src/todo.rs
---

## Scope

Recording the planner's own plan. `todos` is content, and there is no routing at all. The result is
a confirmation.

## Clauses

### TODO-1: there is no routing, because nothing is touched

The plan is shown to the user and reaches nothing else. It is the one tool with no destination.

`verified-by: none`

### TODO-2: an unrecognised status reads as outstanding work

A task is struck through only when it is finished, and anything the driver does not recognise
counts as unfinished.

**Why.** Showing work as done on the strength of a word nobody recognised would misreport what
happened.

`verified-by: bravebot_core::todo::an_unknown_status_is_outstanding_work`
`verified-by: bravebot_core::todo::outstanding_tasks_are_not_struck_whether_started_or_not`
`verified-by: bravebot_tui::render::outstanding_tasks_are_not_struck_through`
`verified-by: bravebot_agent::tools::an_unrecognised_status_shows_as_outstanding`
