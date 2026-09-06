---
id: INCOG
title: A session that leaves nothing behind
status: normative
governs:
  - crates/core/src/incognito.rs
  - crates/agent/src/home.rs
  - crates/tui/src/store.rs
  - crates/tui/src/sessions.rs
  - crates/cli/src/main.rs
---

## Scope

`bravebot --incognito`: a session that runs normally and adds nothing to `~/.bravebot`. What is
ordinarily kept there is [sessions.md](sessions.md): the record of a session and the prompts a
person typed. This is the mode that declines to keep it. The audit trail's contents are
[trace.md](trace.md); this governs only whether it is written down.

The boundary here is the directory this process owns. It is not confinement:
[sandboxing.md](sandboxing.md) is the operating-system boundary, it applies to subprocesses running
code we did not write, and it is a different mechanism answering a different question. A mode that
claimed to stop everything on the machine from writing would be claiming something it cannot
deliver.

## Clauses

<a id="INCOG-1"></a>
### INCOG-1: a prompt typed in an incognito session is not written down

Neither appending one prompt nor rewriting the whole history reaches the file. A history that
existed before the session is left exactly as it was, rather than trimmed or rewritten in passing.

**Why.** The prompt is the thing most worth not keeping. It is what a person typed, in their own
words, and it is the part of a session that most often names something private.

`verified-by: bravebot_tui::incognito::no_prompt_is_written_down`

<a id="INCOG-2"></a>
### INCOG-2: a choice applies to the session and outlives nothing

Choosing a model, a theme or an effort level inside an incognito session takes effect for that
session and is not recorded. The recorded choice a previous ordinary session made is still read at
startup and is still there afterwards.

**Why.** Both halves are the point. Without the first, the mode would be a session in which nothing
can be changed; without the second, entering it once would quietly overwrite settings the person
made deliberately.

`verified-by: bravebot_tui::incognito::a_choice_applies_to_the_session_and_is_not_recorded`

<a id="INCOG-3"></a>
### INCOG-3: no record of the session, and none that it happened

No session record is written, no title is written, and the directory the records would live in is
not created. A session that ran incognito does not appear in the resume list and cannot be resumed,
including by the session itself.

An ordinary session from before may still be resumed and read. It stops being updated for as long
as the incognito session runs, so what is on disk afterwards is what the last ordinary session left
there.

**Why.** An empty directory is a record. It says a session ran, in this project, at this time, which
is most of what the record was for. The directory is therefore asked about before it is created
rather than after.

`verified-by: bravebot_tui::incognito::no_session_record_is_written`
`verified-by: bravebot_tui::incognito::naming_a_session_records_nothing`

<a id="INCOG-4"></a>
### INCOG-4: the audit trail stays in the session

Gate decisions are shown on screen as always and are not appended to a trail on disk.

**Why.** The trail holds no content ([trace.md](trace.md) is what keeps it that way), but it does
hold gate names and paths, which is a record of a session having happened and what it touched.

`verified-by: bravebot_tui::incognito::no_audit_trail_is_written`

<a id="INCOG-5"></a>
### INCOG-5: reading is unchanged

An incognito session reads the settings, the recorded model and theme, the standing instructions,
the skills and the imported credentials, exactly as an ordinary one does. Only writing is refused.

**Why.** A session that could not read its own configuration would not be private, it would be
broken, and one that could not read a credential could not reach a backend at all. This is the same
division a browser's private window makes: the promise is about what survives, not about what the
session may know.

The split is in the source rather than in a convention: `home::directory` is the reading answer and
`home::writable` is the writing one, so a call site says which it is doing and a new write site that
reaches for the wrong one is visible in review.

`verified-by: bravebot_tui::incognito::no_prompt_is_written_down`
`verified-by: bravebot_tui::incognito::a_choice_applies_to_the_session_and_is_not_recorded`

<a id="INCOG-6"></a>
### INCOG-6: asking is one way, and composes with every other way of starting

The mode is engaged once, from the entry point, before anything can have written. There is no way
to turn it off for the life of the process. `--incognito` is taken out of the arguments before
anything dispatches on them, so it combines with `-p`, `--resume`, `--mode` and a bare invocation
alike, and repeating it asks for the same thing once.

**Why.** A flag that could be cleared would oblige every write site to reason about when it was
cleared and by what, and one missed ordering writes the thing the mode exists not to write.

`verified-by: bravebot_core::incognito::engaging_is_idempotent`
`verified-by: bravebot_core::incognito::nothing_is_incognito_until_it_is_asked_for`
`verified-by: bravebot_cli::main::the_incognito_flag_is_taken_out_wherever_it_appears`
`verified-by: bravebot_cli::main::asking_for_incognito_twice_is_asking_once`
`verified-by: bravebot_cli::main::incognito_alone_leaves_the_interactive_session`
`verified-by: bravebot_cli::main::an_invocation_without_the_flag_is_left_alone`

<a id="INCOG-7"></a>
### INCOG-7: importing a subscription is refused rather than silently skipped

`import-leo-creds` in an incognito session reports that it will not run and exits without
registering a device. Forgetting an existing import is still allowed.

**Why.** An import is a write by definition: a credential that did not outlive the session would not
be an import. Doing it and discarding the result would mint a batch on Brave's service that nothing
could ever spend, so the refusal comes before the device is registered. Forgetting is permitted
because removing a stored secret leaves less behind rather than more, which is the direction this
mode points.

`verified-by: none`

<a id="INCOG-8"></a>
### INCOG-8: what the mode does not cover, and says so

Three things still reach the filesystem in an incognito session, each because not doing it would
mean not doing the work:

- **The workspace.** `write_file` and `edit_file` go on editing the project. Those edits are the
  work rather than a trace of it, and a mode that silently declined them would be a broken agent
  rather than a private one. [tools/write-file.md](tools/write-file.md) governs them.
- **Subprocesses.** A program the user asked for runs with the access their own shell would give it
  and may write whatever it likes. Confining it is [sandboxing.md](sandboxing.md)'s question, not
  this one.
- **The editor hand-off.** Opening the input box or the transcript in `$EDITOR` writes a scratch
  file, because there is no way to hand an editor a buffer instead of a path. It goes to the
  system temporary directory rather than `~/.bravebot`, is created `0600` and refuses to reuse an
  existing name, and is unlinked on every path out of the function including the failing ones.

**Why.** A stated limit is worth more than an unstated one. Someone who knows the third of these
can decide not to open an editor; someone who assumed the mode covered it has been misled by their
own tool.

`verified-by: bravebot_tui::editor::the_scratch_file_does_not_outlive_the_edit`
