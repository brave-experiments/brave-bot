---
name: check-spec
description:
  'Check that the implementation matches docs/specs, clause by clause. Runs the
  mechanical pass (clause numbering, verified-by resolution, governs, guards, the
  README table) and then a conformance review of the governed code. Triggers on:
  check spec, /check-spec, make check-spec, does the code match the spec, spec
  conformance, spec drift.'
argument-hint: '[spec-name|spec-id ...] [changed] [strict]'
allowed-tools: Bash(python3 agents/skills/check-spec/*), Bash(make check-spec*)
---

# Check the implementation against the spec

[docs/specs/](../../../docs/specs/README.md) is the source of truth for behaviour. This
skill answers one question about it: **does the code do what the clauses say?**

- **Full run** (default): every spec.
- **Scoped run** (`/check-spec labels routing`, or `/check-spec LABEL`): named specs only.
- **Branch run** (`/check-spec changed`): only the specs governing files this branch
  touched. This is the one to run before a commit.
- `strict` makes warnings fail as well as errors.

`make check-spec` runs the mechanical pass alone, with no model in the loop. It is fast,
deterministic, and safe in CI. The conformance review is what this skill adds.

---

## The one direction this check runs in

**When the code and the spec disagree, the code is wrong.** That is the whole point of the
check, and it is the rule every reviewer prompt repeats.

This skill NEVER edits a spec, adds a clause, softens a clause, or reports "the spec is
unrealistic" as a finding. Specs are closely reviewed by humans and change through a
separate, deliberate process. A run that ends by rewording a clause has verified nothing.

This skill also never fixes the code. It reports. Fixing is a separate task the user asks
for after reading the findings, and mixing the two means the report is written by the same
pass that decided what to change.

---

## Architecture: a file-based pipeline

The heavy data goes through files, never through the main session's context:

1. **check-spec.py** (zero model tokens) runs every mechanical check and writes one prompt
   file per group of clauses. Its stdout is a small JSON pointer.
2. **Reviewers** (subagent tokens only) each read one prompt file, read the governed source
   themselves, and write verdicts to a JSON file.
3. **collect-findings.py** (zero model tokens) merges the mechanical findings with the
   reviewers' verdicts, renders the report, and sets the exit code.

The main session orchestrates. It never reads a spec, a source file, or a verdict.

---

## The job

### Step 1: prepare (zero model tokens)

```bash
python3 agents/skills/check-spec/check-spec.py [spec ...] [--changed] [--strict]
```

Stdout is `{"work_dir": ..., "manifest": ...}`. The mechanical report goes to stderr; print
it, since those findings are already final and the user should see them first.

Parse stdout for `work_dir`.

### Step 2: read the manifest

Read `{work_dir}/manifest.json`. It holds:

- `progress_lines`: print these
- `mechanical_findings`: already decided, already in the report
- `specs`: one entry per spec, each with `reviewer_prompts`, each of those with a
  `prompt_file` and a `results_file` path (paths, never prompt text)

If `specs` is empty, skip to step 4.

### Step 3: launch the reviewers

For every entry in every spec's `reviewer_prompts`, launch a subagent
(subagent_type: `general-purpose`) with exactly this prompt:

```
Read your review instructions from: {prompt_file}
Execute them completely. The instructions name the spec, the clauses in your scope, the governed source, and the guarded symbols.
Write your verdicts JSON to the results file the instructions specify. Do not edit any file.
```

**Launch every reviewer in a single message** so they run concurrently, and launch all of
them: a chunk with no reviewer becomes a `review-incomplete` error, which is the correct
outcome but a wasted run.

Wait for all of them.

**Never write findings yourself.** You have not read the code; the reviewers have. Do not
add, merge, reword, or drop a verdict, and do not decide a reviewer was wrong. If a
reviewer's result is missing or unreadable, the collector says so.

### Step 4: collect (zero model tokens)

```bash
python3 agents/skills/check-spec/collect-findings.py --work-dir "$WORK_DIR" [--strict]
```

Print its output. Exit code 1 means something at severity `error` survived.

Then say, in two or three lines: how many clauses were checked, what failed, and which
finding to look at first. Nothing else. The report is the deliverable.

---

## What each pass decides

| Check | Pass | Fails on |
|---|---|---|
| Clause ids in order, never renumbered, never duplicated | mechanical | error |
| Every clause carries an anchor so it can be linked to | mechanical | error |
| A withdrawn clause says what replaced it | mechanical | error |
| Every clause carries a `verified-by` line | mechanical | error |
| `verified-by` names a `#[test]` that exists, in the module it says | mechanical | error |
| `by-construction` says what makes the clause hold | mechanical | error |
| `verified-by: none` | mechanical | warning |
| `governs` paths exist | mechanical | error |
| `guards` symbols exist | mechanical | error |
| No spec cites another spec's clause ids | mechanical | error |
| The README table lists every spec, with the right id and count | mechanical | error |
| No em-dash | mechanical | error |
| The code does what the clause says | review | error |
| The named tests actually pin the clause | review | warning |
| Nothing untrusted reaches the driver or the planner | review | error |

---

## Scope

Only `docs/specs` and the paths those specs list under `governs`. Code under no spec's
`governs` is ordinary code, reviewed as ordinary code, and out of scope here. Adding a spec
is how a topic becomes review-required, and that is a human's decision.
