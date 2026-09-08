---
name: triage-issues
description:
  'Go through the open issues and decide, against the code, which are already fixed,
  which have gone stale, and which still stand. Closes the fixed ones citing the
  commit that fixed them. Triggers on: triage issues, go through all open issues,
  which issues are already fixed, are these issues still valid, close stale issues,
  issue backlog.'
argument-hint: '[issue-number ...] [label:<name>] [dry-run]'
---

# Triage the open issues against the code

One question per issue: **is this still true of the tree?**

An issue is a claim about the code, written on a day that has passed. Code moved,
sometimes by the very commit that fixed the issue, and nobody closed it. This skill
decides each issue's verdict from the code as it is now, and cites the commit.

- **Full run** (default): every open issue except bot-authored ones.
- **Scoped run** (`/triage-issues 71 82 93`): those issues only.
- **Label run** (`/triage-issues label:security`): issues carrying that label.
- `dry-run` reports and writes nothing to GitHub.

---

## Never touch a bot's issues

Skip any bot-authored issue, and Renovate's in particular. The Dependency Dashboard is
Renovate's own working state: it rewrites the body itself, and it is meant to stay open
forever. Commenting on it or closing it fights a robot that will win, and closing it can
make Renovate open a replacement.

Filter on `is_bot`, not on the login:

```bash
gh issue list --state open --limit 200 --json number,author,title \
  -q '.[] | select(.author.is_bot == false) | .number'
```

`gh` reports Renovate's login as `app/renovate`, with no `[bot]` suffix, so a filter
testing the login for `[bot]` lets it straight through. `is_bot` is the field that
actually holds.

That filter is the intake for a full run. Nothing downstream re-checks it, so anything it
lets through gets worked on.

Renovate's own pull requests are equally out of scope. A dependency bump is not a claim
about this code and there is nothing to verify against the tree.

---

## The three verdicts

| Verdict | Means | Action |
|---|---|---|
| **FIXED** | The thing it asks for exists in the tree now | Comment naming the commit, then close |
| **STALE** | Some claims are false, the ask survives | Comment; rewrite the body only if it is yours |
| **STANDS** | The premise holds | Comment only where a specific claim went stale |

A verdict is a claim about code, so it needs a file, a symbol, or a commit behind it.
"Looks done" is not a verdict.

---

## Method

### Step 1: intake

Get the open issues, filtered as above, and read every body yourself:

```bash
gh issue view N --json title,body,createdAt,labels,author
```

### Step 2: cross-reference the merged pull requests first

Do this before reading any code. It is the cheapest source of truth and it catches the
case no amount of code-reading will explain:

```bash
gh pr list --state merged --limit 200 --json number,title,body,mergedAt,mergeCommit
```

Grep each body for `#\d+`. A pull request saying `Closes #15` that did **not** close #15
is the highest-value find in the whole run, and the usual cause is that it targeted a
feature branch rather than `main`, so GitHub had no reason to honour the keyword. The
work still reached `main`, squashed, under a different SHA.

When a merged pull request carves out follow-up work by number ("the rest of #91 is
#116"), that named issue **stays open**. It is the remainder, not a duplicate.

### Step 3: fan out

One subagent per group of related issues, grouped by subsystem so each reads one area of
the tree. Launch them in a single message. Give each the verdict table, the method notes
below, and this instruction: **read-only, do not edit, close, or comment on anything.**

Ask for at most 120 words per issue: verdict, evidence as file:line or SHA, and for a
stale one, exactly which sentences of the body are now false.

### Step 4: verify every FIXED verdict yourself

A subagent's report says what it believed, not what is in the tree. Before citing any
commit:

```bash
git cat-file -e <sha>^{commit}          # it exists
git merge-base --is-ancestor <sha> origin/main   # it is actually on main
```

A SHA that is not an ancestor of `origin/main` is a feature-branch commit. Citing one
tells a reader to look at work that never shipped.

Then confirm the symbol is really there. Grep for the function, constant, or clause the
verdict rests on. This catches the two failure modes that matter: a subagent that
inferred a fix from a plausible-looking commit message, and one that read a tree in a
different state from yours.

### Step 5: write to GitHub

Comment, then close. Never close with no comment: the citation is the point.

```bash
gh issue comment N --body "$(cat <<'EOF'
...
EOF
)" && gh issue close N
```

---

## Verify against origin/main, not the working tree

Somebody may be working in this checkout while the triage runs, and a rebase moves `HEAD`
under you. A grep that hits a mid-rebase tree reports a symbol as missing when it is
merely not on the current branch, and the finding it produces is confidently wrong.

Read through git, naming the ref:

```bash
git grep -n '<symbol>' origin/main -- crates/
git show origin/main:path/to/file.rs | sed -n '100,140p'
```

Check `git rev-parse --abbrev-ref HEAD` before starting, and leave the working tree as
you found it. This skill changes nothing on disk.

---

## Method notes worth passing to every subagent

**An open issue is not evidence of an unfixed defect.** Issues get fixed and never
closed, sometimes by the same commit that introduced the feature. Verify against the
code, never against the issue's status.

**A similar feature is not the same feature.** The trap is a later feature that shares a
name with what the issue asked for. Read what the issue actually wants: a mode that
withholds writes so a person can review a proposal is not a gate that stops a run and
waits for approval, however alike the two sound.

**Line numbers in a body are from an older tree.** Verify by symbol name and behaviour.
Cite the current line only after seeing it.

**A spec mismatch resolves in two directions.** Either the code changed to obey the
clause, or the clause was reworded, or the behaviour was added to a Known costs list as a
deliberate exception. All three close the mismatch and they mean very different things to
a reader, so say which one happened.

**Counts in a body go stale silently.** Where an issue hardcodes a number, re-measure it.
A count that happens to still be right can hide a list whose membership has largely
turned over, and that is worth reporting.

**Where an issue names a test, open the test.** Check it asserts what the issue wants,
not merely that a test of that name exists.

---

## Rewriting a body: only your own

Check the author before editing:

```bash
gh api user -q .login
gh issue view N --json author -q .author.login
```

**Where the author is somebody else, comment and stop.** Their issue is their account of
a problem, and rewriting it replaces their words with yours under their name. A comment
saying which claims went stale gives a reader everything an edit would, and leaves them
able to disagree. This holds however wrong the body has become.

**Where the author is the configured user**, rewrite a stale body freely, and say in a
comment which claims were removed and why. A body nobody can trust is worse than a
rescoped one, because the next person re-derives the whole thing before finding out.

Rewriting means: drop the false claims, keep the ask, and state what is genuinely left.
Retitle where the title itself is now false.

---

## Prose conventions

Comments and bodies follow the repository's rules in [AGENTS.md](../../AGENTS.md). The two
that this skill breaks most easily:

- **No em-dash.** Anywhere.
- **Never write about your own process.** No "I initially thought", no "verified by
  reading", no narration of what was checked or guessed. State what is true about the
  code, present tense, as though for the first time. Where a correction matters, the
  corrected fact is the whole of it.

Write for somebody who has never seen the conversation that produced the comment. Name
the commit, the symbol, the clause. "This is fixed" is not a citation.

---

## Report

One table: closed, rescoped, annotated, untouched, with counts. Then the closes with
their commits, and anything that needs a human decision.

Say what was **not** covered. An issue nobody was assigned, an ambiguous body whose
intent you had to guess, a verdict resting on one agent's word: each is worth a line.
Silence there reads as coverage.

Two things to surface rather than bury: an issue whose verdict is a live security defect,
and an issue you declined to close despite a subagent calling it fixed. The second is
usually code that exists with nothing pinning it, which is a weaker position than the
verdict suggests.
