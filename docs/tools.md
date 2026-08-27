# Tools

Nine tools. Every argument is either **routing**, which decides what the tool touches and must
be `(T,pub)`, or **content**, which is merely carried and may be untrusted:

| Tool | Routing arguments | Content arguments | Result |
|---|---|---|---|
| `read_file` | `path`, `offset`, `limit` | none | the lines, or a reference |
| `list_files` | `directory`, `pattern` | none | the paths, or a reference |
| `search` | `pattern`, `directory`, `include` | none | matching lines, or a reference |
| `write_file` | `path`, `contents_ref` | `contents` | confirmation |
| `edit_file` | `path`, `replace_all` | `old_text`, `new_text` | confirmation |
| `spawn_processor` | `reads` | `instruction` | a reference |
| `load_skill` | `name` | none | the skill's text |
| `todo_write` | none | `todos` | confirmation |
| `ask_user` | `questions` | none | what you answered, question by question |

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

`load_skill` reads one of the skills named in the system prompt. Its `name` is routing, and it is
promoted the way a read path is, but it is more confined than a read: the name never becomes a
path component, it only selects from a set the driver enumerated before the turn began. A name
holding a traversal matches nothing, because there is no lookup for it to reach. See
[skills.md](skills.md).

`todo_write` has no routing at all: it records the model's own plan, which is shown to the user
and touches nothing.

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

The other bound is on size rather than on rounds. Each round re-sends the whole conversation, so
a long session grows its own request until the server refuses it. Once the last request passes
`BRAVEBOT_CONTEXT_BUDGET` (100,000 tokens by default), the older part of the conversation is replaced,
**in the request only**, by a summary of it; the two most recent exchanges stay word for word, and
`/compact` asks for the same thing by hand. The person's transcript is untouched, and so is the
quarantine, so a reference handed out before a summary still names what it named.

The budget is a guess and has to be. The server reports what a request cost and never what it had
room for, the default model resolves per request, and there is no tokeniser here to count with.

## Who decides what

The model may choose *which* file to read next, because a read cannot change anything and
is confined to the workspace. Every such choice is recorded as a promotion, so an audit
separates the model's decisions from yours.

A write is different: the wrong file destroys work rather than wasting a step. So the model
never gets to decide one. Your approval is what mints a single-use endorsement bound to that
exact path, so an approval cannot be replayed or redirected. Where nobody can be asked, such
as a one-shot `bravebot "..."` run, writes are refused rather than applied unseen.

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

## Asking the user

`ask_user` puts questions to you and hands the model your answers. It is for the planning step,
where the work turns on something only you know: which of two approaches, which file was meant,
whether something is in scope.

One call carries **one to four questions**, and they are put to you one at a time, with the
position shown so you can see how many are left. Each carries a short tag naming what it asks
about, which is how you tell one from the next. For each, you can pick an option, pick several
where the question allows it, answer in your own words, or skip it. Skipping moves to the next
question rather than abandoning the rest: the model is told, question by question, what you
answered and what you passed over.

More than four is refused rather than trimmed. A question quietly dropped would be one the model
was told you had been asked and you never saw.

It is the one tool whose result comes from a person rather than from the workspace, and the only
one with no effect at all. It still has a destination, your screen, and that is what makes the
questions and their options **routing**: they decide what you are shown and therefore what you
can answer. The routing field here is approved by being read. What is drawn is exactly the bytes
the gate checked, nothing re-parses them afterwards, and there is no effect to endorse beyond
the display.

### Asked whole or refused whole

The gate runs once, over a string covering every question in the call. Checking them one at a
time would mean deciding, per question, whether that one is put to you, and which half of a set
survives would then be a decision taken from what is in it. So a call is asked entirely or
refused entirely.

### When asking stops working

The questions are written by the model, so they carry the integrity of the model's context. Once
that context has met something untrusted, the routing gate refuses them. The model is told so and
continues without an answer.

This is not a limitation to work around. A question you were shown may have been written from
bytes an attacker controlled, and choosing among strings an attacker wrote does not make those
strings trustworthy. Treating your keypress as though it did would carry injected text into the
planner's context, which is the one thing this design exists to prevent.

Note what does **not** trip it. Reading a file nobody vouched for leaves the model free to ask,
because that read was quarantined: the planner was handed a reference and never met the bytes, so
nothing in that file could have shaped the question. The context falls when the planner is
*shown* something untrusted, not when a turn reads it. That distinction is pinned by
`a_quarantined_read_does_not_stop_the_planner_asking` in `crates/agent/tests/turn.rs`.

An answer you type is different again: those bytes came from your keyboard, the same source as
the task itself, so they are trusted the way your prompt is. That is a first label rather than an
upgrade, and it is still refused when the question you were answering was not itself trustworthy.

Where nobody can be asked, such as a one-shot `bravebot "..."` run, every question is declined
rather than answered on your behalf. The model is told the reply came from a person, so inventing
one would be worse than not asking. An answer given once is remembered for the session, question
by question, so a model that loops back over the same decision does not make you restate it, and a
set where you have already settled some shows you only the rest.

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

The consequence is that pipes, redirection, `&&`, globbing and `$(...)` are all unavailable **to the
agent**. Each of those is a destination you never saw. Compose stages instead, which is why `run`
takes a pipeline rather than a single program: narrowing output is a stage, not a pipe character.

They are all available to **you**, in shell mode, below. The restriction is on argv the agent chose,
not on your own keyboard.

### Programs are not restricted, output is

There is no allowlist and nothing to configure. `sed`, `awk`, `jq`, `rg`, `gh`, `npm`, anything
installed, all work without being named anywhere. They also run with whatever access your own shell
would give them: `bravebot` does not sandbox them.

The trusted list below is not an exception to this. It decides whether you are *asked* again, never
what may run.

That is deliberate. `git push` needs `~/.ssh`, `npm install` reads `~/.npmrc` and writes
`node_modules`, and the set of programs you might reasonably ask for cannot be listed in advance. A
confinement profile narrow enough to be worth having would break ordinary tools, so instead of
restricting what a program can touch, what is controlled is what its output can *do*.

| | Label | Gate |
|---|---|---|
| Program and arguments | must be `(T,pub)` | you approve the exact argv |
| Standard input | may be untrusted | you approve when it is private |
| Standard output and error | untrusted and private by default | quarantined, per R1 |
| …for a command you vouched for | trusted, still private | you said you trust its output |

Arguments are **routing**: they decide what happens and where it lands, so they may not be derived
from untrusted bytes. Your approval is what makes them trustworthy, and it is bound to that exact
argv so it cannot be reused for a different one.

Standard input is **content**: carried into the process, never consulted. So untrusted data *can* be
fed to a command line. The model names a quarantined reference and the kernel supplies the bytes,
meaning `sed` and `awk` work on a file nobody vouched for without the planner or `bravebot`
itself ever reading it. That is the point of the split: both trusted and untrusted data reach
real tools, and only the routing part has to be trustworthy.

Output is untrusted and private by default, and nothing the model or a stage can say changes that.
A program may print anything, including bytes an earlier stage read out of a file an attacker wrote,
so that is the only label that holds without knowing what ran. The model therefore receives a
reference rather than text and cannot read what it just ran.

Two things change it, and both are you saying so: vouching for the command in advance, below, or
reading one result and letting it through, next.

### Asking to read a result

A run's output is quarantined, so `which`, `find` and `uname` tell the model nothing by
themselves. When it needs the answer it asks for it with `read_output`, naming the reference the
run gave it. You are then shown **the bytes themselves**, with the command that printed them, and
you decide.

```
╭ let the model read this? ────────────────────────────────╮
│Read 1 line  printed by find /Applications -name 'Brave…' │
│                                                          │
│  the model has not seen this. Approving puts it in its   │
│  context, and it will act on it.                         │
│                                                          │
│┃ /Applications/Brave Browser Nightly.app                 │
│                                                          │
│  y let it read this    n keep it back    ctrl-c stop     │
╰──────────────────────────────────────────────────────────╯
```

This is the strongest assertion in the system, and the only one made about bytes rather than about
a prediction: vouching for a command guesses at output that does not exist yet, while this is a
statement about text in front of you. It covers **that one result**. The command is not added to
anything, and the next run asks again.

Only output from `run` can be read this way. A file's worth is the trust map's answer, and `@`,
`/add-dir` and the startup question already give it; a second route would be a way to disagree
with the first.

Errors are worth reading too. A run that failed put its explanation on stderr, and a model that
cannot see it will tell you the command worked.

### Every run asks

There is no read-only category, because there is no way to establish one: `foo --bar` might write to
disk and nothing here can tell. Letting a stage declare itself harmless would only help if the
declaration were honest, and an unprompted write from a stage that claimed otherwise is worse than a
prompt you did not want. So the answer to "does this change anything" is always "assume so".

Private input is a second and independent reason to ask. Untrusted input is fine, because carrying
bytes decides nothing, but private input hands your data to a program, and that releases it somewhere
this policy no longer governs. Trusted-but-private asks too: vouching for what a file contains is not
the same as consenting to send it somewhere.

### Unless you have said "always"

The prompt offers three answers, not two:

```
  y run it    a always    n don't    ctrl-c stop the turn
```

`a` adds that command to the session's trusted list. It grants two things at once, and the prompt
asks for both in those terms, because the second is the one nothing else would tell you:

1. **It runs again unasked**, side effects and all.
2. **What it prints is trusted**, so the model reads it instead of a reference.

The second is your assertion, not a deduction. Nothing here establishes that a command is
side-effect-free or that its output is free of influence, and nothing tries: `git log` prints
commit messages written by whoever contributed to the repository. It is trusted for exactly the
reason a directory you vouched for is trusted, which is that you said so.

So the question `a` is really asking is: *do you take responsibility for what this command does and
for what it prints?* For `make check` in your own tree, usually yes. For `git log` in a repository
with outside contributors, think about it: the commit messages are not yours.

Three properties are worth knowing before pressing it:

- **It is one command, not one program.** The entry is the program *and its exact arguments*.
  Vouching for `git log` says nothing about `git push`, and nothing about `git log --all` either.
- **It is by resolved path.** The prompt shows the binary under the name, and that is what is
  recorded. `$PATH` and aliases decide what `grep` means, so an assertion does not follow the name
  onto a different binary.
- **It belongs to the session.** It is written into the session record and comes back with
  `--resume`, because the person resuming is the person who gave it. A new session in the same
  directory starts with an empty list and asks again, exactly as the trust map does.

In a pipeline, **every** stage must be vouched for or the whole output stays untrusted. An
unvouched stage in the middle is a transformation nobody answered for, and its output is what the
next stage read.

Private input is the exception that stays an exception: those runs ask every time, so `a` is not
offered for them at all.

`/status` lists what you have vouched for. It is the one permission whose whole effect is that
prompts *stop*, so that is where to look if you want to know what you granted.

## Shell mode

Type `!` on an empty prompt and the line becomes a command for your own shell:

```
! ls -la
! git log --oneline | head -20
! cargo build 2>&1 | tail -40
```

The marker is a mode rather than a character: the prompt turns magenta, and what runs is exactly what
you see after it. Backspace over it, or press escape, to get back to a normal prompt. The mode lasts
one command.

This is a **real shell**, `$SHELL` or `/bin/sh`, so globs, `$VAR`, redirection, `&&` and `$(...)` all
work the way they do in your terminal. Everything the previous section says is unavailable applies to
argv the agent chose, not to a line you typed.

**Nothing asks.** The approval prompt exists so that a person endorses argv the *planner* proposed,
because an attacker may have steered it into proposing that. You are the person it would have asked.
Confirming your own keystroke would be theatre, so `! rm -rf build` simply runs.

**What it prints goes to the model**, in full, not as a reference. This is the difference from `run`,
and it is the reason the mode is worth having: after `! cargo test` you can say "fix the first
failure" and the agent has already read the errors.

That output is labelled trusted, and the honest cost should be clear. `! cat notes-from-a-stranger.md`
puts somebody else's words into the planner's context as though they were yours. Nothing inspects the
bytes to catch that, exactly as nothing inspects a directory you vouched for. It is the same
assertion pressing `a` at a run prompt makes, made once for one command: *this is mine, and I take
responsibility for what it prints.* If you would not press `a` for it, do not run it in shell mode:
ask the agent to `run` it instead, and its output will be quarantined.

The agent has no shell and cannot get one. Shell mode is a thing **you** have.

## Pasting a picture

Ctrl-V pastes whatever is on your clipboard, including a screenshot:

```
> why does [Image #1] render like that?
```

The marker is written where the caret is, and the picture goes wherever that text goes. Delete the
marker and the picture is not sent; recall an older prompt and none of them follow it, because the
markers went with the line. Nothing is hidden: what you are about to send is what the prompt says.

**Ctrl-V, not Command-V.** Command-V is your terminal's chord and it never reaches bravebot at all.
The byte stream over a pty has no encoding for that modifier, and what the terminal does instead is
write the clipboard's *text* into the pty, which is why a picture silently arrives as nothing. Ctrl-V
comes through as a byte bravebot can read, so it goes around the terminal and reads the clipboard
itself. Command-V is still the right key for text.

When the clipboard has a picture on it, the prompt says so, and where a terminal sends an empty paste
for Command-V that is read as the picture you meant.

A picture wins over text when the clipboard holds both, which is common: copying an image in a
browser leaves the page's URL behind as the text. Text has another key; a picture has only this one.

On macOS this reads the pasteboard through `osascript`. On Linux it needs `wl-paste` or `xclip`,
which are the same tools copying already uses. Anything over 10 MB is refused rather than sent.

**A pasted picture is trusted, exactly as your prompt is**, and the honest cost is shell mode's. A
screenshot of a hostile page puts a stranger's words into the planner's context as though you had
typed them, and nothing inspects the pixels to catch that. What justifies it is that you chose what
to copy and can see what you pasted. If you would not paste the text of a page into your prompt, do
not paste a picture of it either.

Every paste is named in the audit trail, with its type and size, so `--trace` and Ctrl-T account for
the pictures as well as the words.
