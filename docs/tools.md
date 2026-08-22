# Tools

Five tools. Every argument is either **routing**, which decides what the tool touches and must
be `(T,pub)`, or **content**, which is merely carried and may be untrusted:

| Tool | Routing arguments | Content arguments | Result |
|---|---|---|---|
| `read_file` | `path`, `offset`, `limit` | none | the lines, or a reference |
| `list_files` | `directory`, `pattern` | none | the paths, or a reference |
| `search` | `pattern`, `directory`, `include` | none | matching lines, or a reference |
| `write_file` | `path` | `contents` | confirmation |
| `edit_file` | `path`, `replace_all` | `old_text`, `new_text` | confirmation |

Reads return the content itself when it is trusted and a reference when it is not, per R1.
Writes are silent or shown according to the trust table in [trust.md](trust.md), per R6.

`read_file` pages: it caps at 500 lines and 2000 characters per line, reports the range it
returned, and gives the offset to continue from. A file that is not text is reported as binary
rather than as a decoding error.

`list_files` and `search` take a glob (`*.rs`, `src/**/*.rs`; `*` and `?` do not cross `/`,
`**` does, brace groups are unsupported) and skip version-control and build directories. Both
cap their output and **say so when they do**. Silence there would let the model conclude a file
does not exist when the answer was merely cut off.

`search` matches a literal substring, not a regular expression: a backtracking pattern arriving
through a turn would be a denial-of-service vector. The glob matcher is hand-written and
non-backtracking for the same reason.

`edit_file` replaces an exact passage and refuses rather than guessing when that passage is
missing or occurs more than once, since a guess would change bytes nobody reviewed. It also
refuses if the file changed since it was read, and it requires a **trusted** file, because
locating a passage means comparing text (R3). Use `write_file` for an untrusted file: nothing
is located, and the body is shown in full.

Filenames are content too, since a file can be named to read like an instruction, so an
untrusted listing is quarantined exactly as file contents are. A listing or search that
touches several files is trusted only if every one of them is.

## Who decides what

The model may choose *which* file to read next, because a read cannot change anything and
is confined to the workspace. Every such choice is recorded as a promotion, so an audit
separates the model's decisions from yours.

A write is different: the wrong file destroys work rather than wasting a step. So the model
never gets to decide one. Your approval is what mints a single-use endorsement bound to that
exact path, so an approval cannot be replayed or redirected. Where nobody can be asked, such
as a one-shot `bua "..."` run, writes are refused rather than applied unseen.

## Reviewable writes

When a write is shown, it has to be legible to be worth anything, which is why `edit_file`
exists. Reviewing a whole file body on a terminal is not review, so an edit names the exact
passage to replace and you approve a diff of it. If the passage is missing or occurs more than
once, the edit is refused instead of guessed, because a guess would change bytes you were never
shown. Edits are also refused if the file changed after the model read it, since the diff
you approved would no longer describe what happens.

`edit_file` only works on trusted files. Finding the passage means comparing text, and that is
a decision. On untrusted content it would let file contents decide whether an effect happens.
So an untrusted file is refused rather than edited blind; trust the path, or replace the whole
file with `write_file`, where nothing is located and the body is shown to you in full.

## Running programs

`run` takes a **pipeline of stages**, each a program name and a list of arguments. There is no
command string anywhere, and no shell:

```
run { pipeline: [
  { program: "git", args: ["log", "--oneline", "-50"] },
  { program: "sed", args: ["-n", "1,10p"] }
]}
```

This is what makes command execution admissible when a shell is not. A shell string is destination
and payload at once, so there is nothing in it a person could approve on its own, and a parser that
tried to work out what it means would be racing a shell it does not control. An argument list has
no such problem: `; rm -rf /` in an argument is one argument and stays one, because nothing ever
splits it. What you approve is what runs, verbatim.

The consequence is that pipes, redirection, `&&`, globbing and `$(...)` are all unavailable. Each
of those is a destination you never saw. Compose stages instead, which is why `run` takes a
pipeline rather than a single program: narrowing output is a stage, not a pipe character.

### No list of permitted programs

There is nothing to configure and no allowlist to maintain. `sed`, `awk`, `head`, `jq`, `rg`, `gh`,
anything installed, all work without being named anywhere.

What bounds a stage is not its name but the confinement it runs under, and the operating system
enforces that. Each stage declares whether it needs to write or to reach the network, and that
declaration selects a sandbox profile that permits nothing else. A stage that claimed to be
read-only and then tried to write does not get a silent write; it gets a denied one. So a
misdeclaration fails rather than escaping, and no list of trustworthy programs is required.

### What goes in and what comes out

| | Label | Gate |
|---|---|---|
| Program and arguments | must be `(T,pub)` | promoted when confined, endorsed by you for an effect |
| Standard input | may be untrusted | asks you when private |
| Output | always untrusted, always private | quarantined, per R1 |

Arguments are **routing**: they decide what happens. So they may not be derived from untrusted
bytes, which is the injection this design exists to prevent.

Standard input is **content**: it is carried into the process, never consulted. So untrusted data
*can* be fed to a command line. The model names a quarantined reference and the kernel supplies the
bytes, meaning `sed` and `awk` work on a file nobody vouched for without the planner or `bua` itself
ever reading it. This is the point of the split: both trusted and untrusted data reach real command
line tools, and only the routing part has to be trustworthy.

Output is **always** untrusted and private. Every stage, no exceptions, nothing to configure. A
program can emit anything, including bytes an earlier stage read out of a file an attacker wrote, so
that is the only label that holds without knowing what ran. The model therefore gets a reference
rather than the text, and cannot read what it just ran. Whether some narrower class of output could
ever be trusted is an open question rather than a settled one, tracked as an issue.

### Private input asks, even when nothing changes

Integrity and confidentiality gate differently. Untrusted input is fine, because carrying bytes
decides nothing. Private input is not, because handing it to a subprocess releases it somewhere the
policy no longer governs.

So a pipeline fed private content asks for your approval even when every stage is confined and
nothing is being written. It would be easy to reason that a stage with no network and no writes
could not leak anything, and that reasoning is exactly what this design declines to rely on: the
process had the bytes, and you decide instead. Trusted-but-private asks too. Vouching for what a
file contains is not the same as consenting to send it somewhere.
