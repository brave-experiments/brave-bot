---
name: release-notes
description:
  'Write the release notes for a version: one section per version, new features
  first, then changes, then fixes, and only the things a person using bravebot
  would notice. Reads the commits since the last release tag, or since the last
  version bump when there is no tag. Triggers on: release notes, changelog,
  notes for v1.2.3, what changed since the last release, /release-notes.'
argument-hint: '[version] [since <tag|ref>]'
allowed-tools: Bash(git log:*), Bash(git tag:*), Bash(git show:*), Bash(git describe:*),
  Bash(git add:*), Bash(git commit:*), Read, Write, Edit
---

# Release notes for a version

The deliverable is one markdown section: a version heading and a flat list of bullets. It
goes in the GitHub release body, and it is prepended to `CHANGELOG.md` and committed,
every time and without being asked.

**A bullet is a thing a person using bravebot would notice.** Everything else stays out.
Thirty commits routinely become five bullets, and a release with nothing user-visible in it
gets told so rather than padded.

---

## The format

```markdown
## [0.2.0](https://github.com/brave-experiments/bravebot/releases/tag/v0.2.0)

 - Added `/loop`, which repeats a prompt on an interval or at a pace each turn sets. ([#66](https://github.com/brave-experiments/bravebot/issues/66))
 - Added ctrl-r, which searches the prompts already sent in this session.
 - Added `/cd`, which moves a session to another directory and carries its trusted paths with it.
 - Changed the Bedrock environment variable to `BRAVEBOT_USE_BEDROCK`. The old name is no longer read.
 - Fixed the prompt being drawn after the checks that run before a turn, which left it blank for a moment.
```

Exactly that, clause by clause:

- The heading is `## [<version>](<url of the release tag>)`, with no `v` in the link text
  and a `v` in the tag it points at. One blank line under it.
- Every bullet is one line: one leading space, `- `, then one sentence saying what the
  change is, and at most one more if a person needs a caveat or how to reach it. No blank
  lines between bullets, no nesting, no sub-headings, no bold labels.
- **Bottom line first, in plain language, and short.** The first clause names the thing a
  person got. Whatever qualifies it comes after: how it is reached, what it costs, what it
  does not do. A reader skimming the first half of every bullet should come away with the
  release. Prefer the ordinary word to the precise one, and cut any clause that would not
  change what somebody does next.
- Order is new features first, then changes, then fixes. Inside each group, most impactful
  first: what most people will use, or what they will find most interesting, leads. A new
  capability outranks a preference panel; a preference panel outranks a message that got
  clearer. The opening verb is what tells a reader which group a bullet is in, so there is
  nothing to label.
- An issue link goes after the sentence's period, in parentheses:
  `([#84](https://github.com/brave-experiments/bravebot/issues/84))`. Only when a commit
  named the issue.
- Sections stack newest first, and a released section is never edited afterwards.

---

## Step 1: find the range

The notes cover everything between the last release and the version in the tree.

```sh
current="$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([0-9]+\.[0-9]+\.[0-9]+)".*/\1/p' Cargo.toml | head -n 1)"

# The newest release tag reachable from HEAD, ignoring one that names the version being
# written: that release is this one, not the one before it.
base="$(git tag --merged HEAD --list 'v*' --sort=-v:refname | grep -vx "v$current" | head -n 1)"

# No tag, so fall back to the commit that last set a *different* version. That commit
# opened the release being written, so the release before it ended there.
if [ -z "$base" ]; then
  base="$(git log --format='%H %s' --grep='[Bb]ump version to' HEAD |
          grep -v " to $current\$" | head -n 1 | cut -d' ' -f1)"
fi
```

- `since <ref>` in the arguments overrides all of this.
- A version in the arguments overrides `$current` for the heading.
- No tag and no bump commit, with a version that has moved anyway: no commit message says
  where, so `git log -S'version = "' --format='%H %s' -- Cargo.toml` finds it.
- No tag, no bump commit, and a version that has never moved: this is the first release.
  The range is the whole history, and say so when handing the notes over.

Print the range you settled on (`v0.1.1..HEAD`, 35 commits) before going further. Getting
it wrong silently is how a bullet from a shipped release ends up in the next one.

## Step 2: read the commits

```sh
git log --no-merges --format='%h%n%s%n%b%n---' "$base..HEAD"
```

Read the bodies, not only the subjects. The body is where a commit says what a person
gets from it, and where `Closes #84` or `Part of #68` names the issue to link.

## Step 3: decide what a user would care about

| Include | Leave out |
|---|---|
| A new command, flag, key binding, or configuration key | Refactors, renames, and internal structure |
| Behaviour a person can invoke or will see happen | Tests, specs, `AGENTS.md`, docs about internals |
| A changed default, or something that now works differently | CI, build files, release plumbing |
| A fix for something a person on the last release would hit | A fix for a feature that has not shipped yet |
| A breaking change: a renamed flag, a dropped option, a moved file | Dependency bumps with no visible effect |
| Install, platform, and packaging changes | The version bump commit itself |

Two rules that decide most of the hard cases:

- **A bullet is one user-visible change, not one commit.** Five commits that built one
  feature are one bullet. The feature's own follow-up fixes, landed in the same release,
  fold into it or disappear: to a reader, that bug never existed.
- **A fix is only news if the thing it fixes shipped.** Check whether the broken behaviour
  is reachable from `$base`. If it is not, there is nothing to tell anyone.

A breaking change is always in, however small, and it leads the list.

## Step 4: write the bullets

- Start with a past-tense verb: `Added`, `Implemented`, `Improved`, `Changed`, `Updated`,
  `Removed`, `Fixed`.
- Say what the person gets, in their vocabulary. No crate names, no function names, no
  module paths, no commit hashes, no conventional-commit prefixes.
- Name a tool, command, key or flag a person types, in backticks. A capability described
  without its name leaves them unable to reach it.
- Lead with the capability, not the mechanism. "Added code navigation through a language
  server" before the eight operations it supports.
- Never invent an issue number, and never link one a commit did not name.

```
Bad:   Added `/cd` to `bravebot-tui`, moving `Session::workspace` and rebuilding the trust map.
Good:  Added `/cd`, which moves a session to another directory and carries its trusted paths with it.

Bad:   fix(bedrock): ask the AWS CLI whether a session is good once, not every turn
Good:  Fixed the delay before every turn on Bedrock, caused by re-checking the AWS session each time.

Bad:   Added a language server tool, so a turn can ask where a symbol is defined, what
       references it, what a hover says, the symbols in a file or across the workspace, an
       implementation, and which functions call which. The server runs with your own access
       once you agree to it, and its index is cached under `~/.bravebot` so a later session
       does not wait for it again.
Good:  Added code navigation through a language server: jump to a definition, find
       references, read a hover, list the symbols in a file or the workspace, and follow a
       call in either direction. The server runs with your access once you allow it.
```

## Step 5: where it goes

Print the section in the reply. That is what to paste into the GitHub release body.

Then put it in `CHANGELOG.md` at the repository root, every time: prepend it above the
newest section, and create the file with this section alone when it does not exist. A
released section is never edited, so a section already there for this version is replaced
whole rather than appended to.

Commit that file on its own, with `updated changelog for version <version>` as the whole
message. Nothing else goes in the commit: the notes describe a range that ends at HEAD, so
a code change staged alongside them is a change they do not cover.

## Before handing it over

- Every bullet traces to a commit in the range, and every user-visible commit in the range
  is in a bullet or was deliberately dropped.
- No bullet describes a fix to something that never shipped.
- Reading only the first clause of every bullet still gives a reader the release, and the
  bullet a reader would care about most is the first one.
- No em-dash, anywhere. A comma, a colon, or two sentences does the job.
- The heading version matches `Cargo.toml` and `package.json`. `make github-release` refuses
  when those two disagree, and names the tag from `Cargo.toml`, so a mismatch here is a
  mismatch worth mentioning now.
