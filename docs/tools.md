# Tools

Six tools. Every argument is either **routing**, which decides what the tool touches and must
be `(T,pub)`, or **content**, which is merely carried and may be untrusted:

| Tool | Routing arguments | Content arguments | Result |
|---|---|---|---|
| `read_file` | `path`, `offset`, `limit` | none | the lines, or a reference |
| `list_files` | `directory`, `pattern` | none | the paths, or a reference |
| `search` | `pattern`, `directory`, `include` | none | matching lines, or a reference |
| `write_file` | `path`, `contents_ref` | `contents` | confirmation |
| `edit_file` | `path`, `replace_all` | `old_text`, `new_text` | confirmation |
| `spawn_processor` | `reads` | `instruction` | a reference |

Reads return the content itself when it is trusted and a reference when it is not, per R1.
Writes are silent or shown according to the trust table in [trust.md](trust.md), per R6.

`read_file` pages: it caps at 500 lines and 2000 characters per line, reports the range it
returned, and gives the offset to continue from. A file that is not text is reported as binary
rather than as a decoding error.

A read whose result would be quarantined does not open the file. There is nothing to show the
planner, so the slot holds the path and the reference says the size and that nothing has looked
yet; the file is read when a processor or a write needs the bytes, and never if neither does.
That matters because most of what an agent reads in a directory nobody vouched for is a file it
turns out not to want. What is deferred is only the reading: the path is checked, the file is
confirmed to be there and to be text, and the label is fixed from the trust map, all at the
moment the planner asks. When the bytes are finally read the path is checked again, so a file
that lost its trust in between is read as untrusted rather than at the label its reference was
issued with.

`list_files` and `search` take a glob (`*.rs`, `src/**/*.rs`; `*` and `?` do not cross `/`,
`**` does, brace groups are unsupported) and skip version-control and build directories. Both
cap their output and **say so when they do**. Silence there would let the model conclude a file
does not exist when the answer was merely cut off.

`search` matches a literal substring, not a regular expression: a backtracking pattern arriving
through a turn would be a denial-of-service vector. The glob matcher is hand-written and
non-backtracking for the same reason.

`write_file` takes either the `contents` to write or a `contents_ref` naming quarantined
content that becomes the whole file, never both. The reference is routing, since it decides
which bytes the write carries, and it is a name the driver handed out rather than anything
derived from content: the worst a wrong one can do is put the wrong quarantined bytes into a
path that still had to be endorsed on its own.

`edit_file` replaces an exact passage and refuses rather than guessing when that passage is
missing or occurs more than once, since a guess would change bytes nobody reviewed. It also
refuses if the file changed since it was read, and it requires a **trusted** file, because
locating a passage means comparing text (R3). For an untrusted file, put the change through a
processor and write the reference it returns: nothing is located, and the body is shown to you
in full.

Filenames are content too, since a file can be named to read like an instruction, so an
untrusted listing is quarantined exactly as file contents are. A listing or search that
touches several files is trusted only if every one of them is.

Such a listing comes back as **one reference per entry**, not one for the listing. That is what
keeps the quarantine from being a dead end: a reference is an address as well as a document, so
the planner passes `path_ref` where it would have typed a path.

```
list_files "."              →  [ref:1] an entry in "." (not read yet, (U,priv))
                               [ref:2] an entry in "." (not read yet, (U,priv))
spawn_processor reads=[ref:2]  →  [ref:4]
     instruction="if this sets the movement speed, fix …; else return it unchanged"
write_file path_ref=ref:2 contents_ref=ref:4
     ↳ always asks:  Overwrite game.js   +1 -1
```

The planner is never told a filename, at any point. The person approving the write is, which is
where it belongs: they own the directory, and they are the only party who can say whether that
file should be rewritten. A write to a `path_ref` is shown every time, even where the trust table
would not ask, because the approval is the only moment the path is visible to anybody. A
reference that names no file, which is anything a processor produced, is refused as a
destination.

Reading through a reference is the ordinary confined-read promotion: the model already chooses
which file to read next, and this only changes where the name came from. `search` still returns
one reference for the whole result, so its hits are not addresses yet.

## Processors

An agent that may not read a file also cannot change it. `edit_file` refuses on an untrusted
file, because matching a passage is a comparison, and a whole-file `write_file` would need a
body the planner could only have guessed at. That would leave the agent able to answer questions
about a repository nobody vouched for and unable to do any work in one.

`spawn_processor` closes that gap. It starts a second model instance holding nothing:

| | |
|---|---|
| Tools | none, and the request carries no tool list at all |
| Memory | none: the messages are built from nothing each time |
| Conversation | one request, one reply, no loop to steer |
| Reads | exactly the references named in `reads`, and nothing else |
| Writes | one new reference, and nothing else |
| Label of that reference | computed by taint over the inputs, **before** it runs |

So a processor is the only component in the system that reads quarantined content, and it is
the one component that can do nothing with what it reads. Injected text in its input can change
the bytes in a slot nobody has read. It cannot reach a routing field, because everything the
processor produces is quarantined, and it cannot persist, because the processor is gone when the
call returns.

The usual shape is three calls:

```
read_file  src/config.py        →  [ref:1] 6 lines, (U,priv), quarantined
spawn_processor reads=[ref:1]   →  [ref:3] 24 lines, (U,priv), quarantined
           instruction="add error handling; return the whole file"
write_file path=src/config.py contents_ref=ref:3
```

Nothing in that sequence reads the file. The planner sees two line counts and two names; the
driver carries bytes it cannot open; the user sees the diff and approves it. The instruction is
the only thing steering the processor, and it comes from the planner, whose context holds
nothing an attacker wrote.

An instruction can ask the processor to decide as well as to rewrite, and that is what makes
the mechanism useful rather than merely sound. The planner has not seen the file, so it has no
edit to hand over; what it has is the task. So the instruction carries the task: "this is
game.js; if it sets the movement speed, fix the bug that makes it double each frame; if it does
not, return the document exactly as it was." Where several files could be the one, each is read
into its own slot, transformed with the same instruction, and written back to the path it came
from.

Nothing about the guarantee moves when it does. The processor's judgement decides which bytes
land in a slot nobody has read, and no more than that: the destination is a path the planner
named, the write is one a person approves, and a file the processor chose to leave alone is
written back byte for byte. What it costs is a write, and therefore an approval, for each
candidate rather than for the one that changed.

One thing this changes and should be said plainly: a processor is a model call, so an untrusted
file's contents now reach the backend when the agent is asked to work on one, where before they
would have stayed on the machine. The destination is the one a trusted directory has always sent
its files to. What is new is only that the reader holds nothing.

What a processor is **not** is an operating-system sandbox. There is no untrusted code involved:
the call is made by the same trusted driver that makes every other call. The confinement is the
capability set, which is empty, and the label on the output, which nothing in the processor
chooses.

Writing quarantined content into a file is a move inside the workspace rather than a release out
of it, which is why a private slot may become a file body when it may not become a network body
or a command line. The trust map then records that path as untrusted, so reading it back does
not launder it.

## What bounds a turn

A turn may make 40 rounds of tool calls. On the fortieth the tools are taken away rather than the
turn ended: the next request offers none, the planner is told it has none left, and it answers
with what it has. A call it asks for anyway is dropped rather than run.

This is not a safety property. A gate refuses on the thousandth round what it refuses on the
first, and nothing here gets more dangerous for running longer. It is a bound on futility, and
the case that needs it is a directory nobody vouched for: every listing comes back as a
reference, no filename can be learned from one, and a planner looking for a file it cannot name
will try glob after glob for as long as anyone lets it. Forty is well past what real work in a
large repository takes, and well short of an afternoon.

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
So an untrusted file is refused rather than edited blind; trust the path, or send the change
through a [processor](#processors) and write the whole file, where nothing is located and the
body is shown to you in full.

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

### Programs are not restricted, output is

There is no allowlist and nothing to configure. `sed`, `awk`, `jq`, `rg`, `gh`, `npm`, anything
installed, all work without being named anywhere. They also run with whatever access your own shell
would give them: `bua` does not sandbox them.

That is deliberate. `git push` needs `~/.ssh`, `npm install` reads `~/.npmrc` and writes
`node_modules`, and the set of programs you might reasonably ask for cannot be listed in advance. A
confinement profile narrow enough to be worth having would break ordinary tools, so instead of
restricting what a program can touch, what is controlled is what its output can *do*.

| | Label | Gate |
|---|---|---|
| Program and arguments | must be `(T,pub)` | you approve the exact argv |
| Standard input | may be untrusted | you approve when it is private |
| Standard output and error | always untrusted, always private | quarantined, per R1 |

Arguments are **routing**: they decide what happens and where it lands, so they may not be derived
from untrusted bytes. Your approval is what makes them trustworthy, and it is bound to that exact
argv so it cannot be reused for a different one.

Standard input is **content**: carried into the process, never consulted. So untrusted data *can* be
fed to a command line. The model names a quarantined reference and the kernel supplies the bytes,
meaning `sed` and `awk` work on a file nobody vouched for without the planner or `bua` itself ever
reading it. That is the point of the split: both trusted and untrusted data reach real tools, and
only the routing part has to be trustworthy.

Output is **always** untrusted and private. Every stage, no exceptions, nothing that changes it. A
program may print anything, including bytes an earlier stage read out of a file an attacker wrote, so
that is the only label that holds without knowing what ran. The model therefore receives a reference
rather than text and cannot read what it just ran, which is a real limitation. Whether some narrower
class of output could ever be trusted is an open question rather than a settled one.

### Every run asks

There is no read-only category, because there is no way to establish one: `foo --bar` might write to
disk and nothing here can tell. Letting a stage declare itself harmless would only help if the
declaration were honest, and an unprompted write from a stage that claimed otherwise is worse than a
prompt you did not want. So the answer to "does this change anything" is always "assume so".

Private input is a second and independent reason to ask. Untrusted input is fine, because carrying
bytes decides nothing, but private input hands your data to a program, and that releases it somewhere
this policy no longer governs. Trusted-but-private asks too: vouching for what a file contains is not
the same as consenting to send it somewhere.
