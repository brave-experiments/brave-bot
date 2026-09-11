# Specs

Each file in this directory is a mini-spec for one topic within the bravebot system.
Spec changes, additions, and removals are closely reviewed by humans.

If a mini-spec disagrees, that is a bug in the spec that should be fixed.

Specs have automation attached which verifies that there is coverage of functionality and also
that functionality matches specs. Bugs are posted for any problems found.

## The specs

| Spec | Id | Clauses | Topic |
|---|---|---|---|
| [labels.md](labels.md) | `LABEL` | 9 | the lattice, taint, who may read what, and how a first label is assigned |
| [routing.md](routing.md) | `ROUTE` | 7 | where an effect may land and what may decide it |
| [trust-map.md](trust-map.md) | `TRUST` | 13 | which paths the user vouched for, what a write does to that record, and how long an answer lasts |
| [permissions.md](permissions.md) | `PERM` | 12 | rules written in advance about what to ask about and what to refuse |
| [processors.md](processors.md) | `PROC` | 9 | the one component that reads untrusted content, and what it may do with it |
| [delegation.md](delegation.md) | `DELEGATE` | 17 | a second planner, narrower than the first, and what crosses back from one |
| [turns.md](turns.md) | `TURN` | 4 | how long a turn may go on, what happens when it does not stop, and what is said when it produces nothing or checks nothing |
| [prompting.md](prompting.md) | `PROMPT` | 10 | every moment the system stops and puts something to a human, and what an answer grants |
| [permission-modes.md](permission-modes.md) | `MODE` | 10 | a standing answer to those prompts: accepting edits, planning, or asking about nothing at all |
| [naming-files.md](naming-files.md) | `NAME` | 7 | writing `@path` in a prompt: what it puts into the turn and what it vouches for |
| [pasting.md](pasting.md) | `PASTE` | 9 | what Ctrl-V puts into a turn, text or picture, and on what footing |
| [dropping.md](dropping.md) | `DROP` | 9 | what dragging a file onto the terminal puts into a turn, and on what footing |
| [shell-mode.md](shell-mode.md) | `SHELL` | 5 | the `!` prompt: a line the user typed, and why the planner can never reach it |
| [skills.md](skills.md) | `SKILL` | 10 | `AGENTS.md` and skills: what a skill file is and what each source is trusted for |
| [instructions.md](instructions.md) | `INSTR` | 9 | which instruction files are looked for, where, in what order, and where what they say ends up |
| [cli.md](cli.md) | `CLI` | 11 | running without the interactive interface: one-shot tasks, piped input, and `doctor` |
| [manifest.md](manifest.md) | `MANIFEST` | 9 | plan the whole run first, then execute it with no model in the control path |
| [terminal-input.md](terminal-input.md) | `INPUT` | 30 | what the user types into: the box, the keys, and where a terminal's own limits show through |
| [commands.md](commands.md) | `CMD` | 7 | a line beginning with `/`: where one may come from, when a line is one, and what it does to the line |
| [terminal-transcript.md](terminal-transcript.md) | `VIEW` | 19 | what is drawn back: the transcript, a resumed session, and how content reaches the screen |
| [watching.md](watching.md) | `WATCH` | 20 | work done outside the transcript: a delegate's own lines, what a command printed, a question asked beside it, and the mode Ctrl-L opens over them |
| [scroller.md](scroller.md) | `SCROLL` | 9 | reading back through what happened: the mode Ctrl-O opens over the transcript, and the keys inside it |
| [premium-credentials.md](premium-credentials.md) | `PREM` | 9 | importing a Leo Premium subscription and spending its credentials |
| [sandboxing.md](sandboxing.md) | `SANDBOX` | 5 | operating-system confinement for processes running code we did not write |
| [mcp.md](mcp.md) | `MCP` | 8 | tools that come from outside this repository, and what they are allowed to do |
| [network-egress.md](network-egress.md) | `NET` | 6 | every request that leaves this process, and what comes back |
| [backends.md](backends.md) | `BACKEND` | 29 | which service answers a request, and what a person may choose between |
| [compaction.md](compaction.md) | `COMPACT` | 10 | shortening a long conversation into a summary of itself, in the request only |
| [loop.md](loop.md) | `LOOP` | 13 | sending one prompt again and again until somebody stops it |
| [goal.md](goal.md) | `GOAL` | 16 | one condition a person set, judged after every turn, until it holds |
| [sessions.md](sessions.md) | `SESSION` | 20 | what is kept between runs: the record of a session, the questions asked beside it, and the prompts a person typed |
| [state-directory.md](state-directory.md) | `STATE` | 2 | `~/.bravebot`, and who on the machine may read what is written into it |
| [incognito.md](incognito.md) | `INCOG` | 8 | a session that runs normally and adds nothing to `~/.bravebot` |
| [trace.md](trace.md) | `TRACE` | 6 | what is recorded about every decision the system makes, and what that record may contain |
| [localization.md](localization.md) | `LOCALE` | 7 | every word said to a person, and which of them change with the reader's language |
| [layering.md](layering.md) | `LAYER` | 3 | which crate is allowed to do what |
| [releases.md](releases.md) | `RELEASE` | 12 | what names a version, what starts a release, and what an installer trusts about what it fetched |
| [updates.md](updates.md) | `UPDATE` | 10 | learning that a newer version is out, and the line that installs it |

## The tools

One spec per tool. [tools/tool-surface.md](tools/tool-surface.md) is the table of all of them and
the routing-versus-content split they share.

| Spec | Id | Clauses | Tool |
|---|---|---|---|
| [tools/tool-surface.md](tools/tool-surface.md) | `TOOL` | 3 | the surface every tool shares |
| [tools/read-file.md](tools/read-file.md) | `READ` | 5 | `read_file` |
| [tools/list-files.md](tools/list-files.md) | `LIST` | 5 | `list_files` |
| [tools/search.md](tools/search.md) | `SEARCH` | 7 | `search` |
| [tools/lsp.md](tools/lsp.md) | `LSP` | 10 | `lsp` |
| [tools/write-file.md](tools/write-file.md) | `WRITE` | 4 | `write_file` |
| [tools/edit-file.md](tools/edit-file.md) | `EDIT` | 4 | `edit_file` |
| [tools/spawn-processor.md](tools/spawn-processor.md) | `SPAWN` | 3 | `spawn_processor` |
| [tools/spawn-agent.md](tools/spawn-agent.md) | `AGENT` | 5 | `spawn_agent` |
| [tools/run.md](tools/run.md) | `RUN` | 16 | `run` |
| [tools/command-line.md](tools/command-line.md) | `CMDLINE` | 16 | `run`'s command line, compiled rather than interpreted |
| [tools/read-output.md](tools/read-output.md) | `OUTPUT` | 2 | `read_output` |
| [tools/fetch-url.md](tools/fetch-url.md) | `FETCH` | 5 | `fetch_url` |
| [tools/load-skill.md](tools/load-skill.md) | `LOAD` | 3 | `load_skill` |
| [tools/todo-write.md](tools/todo-write.md) | `TODO` | 2 | `todo_write` |
| [tools/schedule-next.md](tools/schedule-next.md) | `SCHED` | 5 | `schedule_next` |
| [tools/ask-user.md](tools/ask-user.md) | `ASK` | 8 | `ask_user` |

Topics with no spec yet are ordinary code. Adding one is how a topic becomes review-required.

## Format

Front matter, then numbered clauses. Everything outside a clause is commentary and binds nobody.

- **`id`** is a short prefix. Clause ids are `PREFIX-N`, allocated in order and never reused and
  never renumbered, because a commit message, an issue, and a test name all point at one. A
  withdrawn clause stays, marked withdrawn, and says what replaced it.
- **Every clause carries an anchor**, `<a id="PREFIX-N"></a>` on the line directly above its
  heading, so `labels.md#LABEL-3` is a link that keeps working. The anchor GitHub generates from
  a heading contains the title, so it breaks the moment somebody improves the wording, which is
  the moment an issue or a commit pointing at that clause most needs the link to survive.
- **`governs`** lists the paths this spec decides. A diff touching one of them is reviewed against
  this file. Anything under no spec's `governs` is ordinary code and reviewed as such.
- **`guards`** lists symbols whose every use is review-required.
- **`verified-by:`** lines name the tests that pin a clause, as `crate::module::test_name`. The
  coverage check reads them, fails when a name does not resolve to a test that exists, and posts a
  bug for any clause whose value is `none`. `by-construction` is for a clause nothing can execute,
  such as a crate having no dependencies, and says in brackets what makes it hold.

A clause is one rule, stated so that a reader can tell whether a given diff obeys it. If a clause
needs an "and" it is usually two clauses. A set of rules that reads best as a table is one clause with a
table in it, never a clause id per row: ids must be greppable one to a heading.

**Each spec stands alone.** A clause id is only ever cited inside its own file, where a reader can
scan for it. Never cite another spec's clause: somebody landing here first has no idea what
`TRUST-5` is, and chasing an id across files to understand one rule is how a set of small documents
becomes worse than one big one. Where a clause leans on something another spec owns, state the fact
plainly in a few words. "The same 'most specific wins' rule the trust map uses" costs a line and
needs no second tab.

**Point at documents, not at ids.** When a whole topic lives elsewhere, link the file by name:
`[trust-map.md](trust-map.md)`, not `` `TRUST` ``. A bare prefix means nothing to a first-time
reader, and it cannot be clicked.

**Clauses describe behaviour, not implementation.** Say what happens and why, never which function
or type does it. A clause naming a symbol becomes wrong the next time somebody renames one, and it
pins the code that exists rather than the behaviour the code owes. Identifiers belong in `governs`,
`guards` and `verified-by`, where tooling reads them and a rename is caught by the check rather
than by a reader noticing.
