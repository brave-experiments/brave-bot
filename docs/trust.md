# Trusted directories

At startup you are asked whether you trust the working directory. Trusting it means files
there are read as **trusted**, which is what lets ordinary work proceed without a prompt for
every edit. Decline and nothing is trusted, so every write is shown to you.

Trust is per path, and the **most specific rule wins**, so a trusted project can contain an
untrusted subtree, and that subtree can contain a trusted path again.

A prompt asks you one thing: **may this path stop being trusted?** That is the only
consequence a later step cannot undo, since a path recorded as untrusted can no longer be
examined or edited.

The table below is the specification.

| data | destination | prompt? | effect on the trust map |
|---|---|---|---|
| trusted | trusted | no | unchanged |
| untrusted | trusted | **yes** | that path becomes untrusted |
| trusted | untrusted | no | that path becomes trusted |
| untrusted | untrusted | no | unchanged |
| either | *never mentioned* | **yes** | that path takes the data's trust |

Writing trusted data never asks. For data to be trusted the turn must have observed nothing
untrusted, so there is no attacker-influenced byte in it, and the destination only gains trust,
never loses it.

The last row is why a path nobody has mentioned differs from one you deliberately marked
untrusted: the first has no decision behind it, so the first write there is the moment to ask.
This is also what makes declining at startup meaningful: with nothing vouched for, every write
is shown.

The second row is what closes the obvious hole. If untrusted data, meaning anything derived
from the web or from a file outside a trusted path, is written into a trusted directory, that
file is recorded as untrusted. Reading it back returns untrusted data. Otherwise a round trip
through the filesystem would launder injected text into trusted input, and the trust map would
become a bypass for the gate it exists to support.

Marking is always per file, never per directory: one untrusted file does not taint its
siblings.

## The map belongs to the session, not the directory

**You are asked every time a session starts.** Whatever you or anyone else answered in this
directory before makes no difference. The question grants standing permission, so a launch that
skipped it because someone said yes last week would be granting that permission on behalf of a
user who was never asked, and trust assumed from silence is not trust granted.

Resuming with `--resume` is the one case that does not ask. That is not an exception to the rule
above: the map comes out of the record of the session you picked, so the answer being honoured is
the one you gave that session. It carries the rules that session's writes recorded along with it,
which is what stops a resumed turn reading back a file an earlier turn of the same session
poisoned. A record from before maps were kept has none, and is asked about.

`/clear` **does** ask, because it begins a session and every session is asked. The map goes with the
context it was built alongside, and so do the directories `/add-dir` opened under it: opening one is
a grant, and leaving a tree reachable once nothing vouches for it would outlive the answer that
allowed it. Leaving at the question ends the session, as it does at startup.

The consequence is worth stating plainly. A file that one session recorded as untrusted is **not**
remembered by the next session started fresh in that directory: say yes to the directory and it is
read as trusted again. Within a session, and across a resume of it, the second row of the table
holds. Across a fresh start it does not, because a fresh start has no memory to hold it in. If a
file holds content you do not trust, the answer is to say no to the directory, or to not leave it
there.

## Naming a file with `@`

Writing `@src/main.rs` in a prompt puts that file's **contents into the turn as trusted input**, and
typing `@` offers what is in the workspace so the choice is an informed one. It is the same thing
`--file` does on the command line, and it is trusted for the same reason: you typed the path, and
sending the line is the grant. Nothing a model said chose the file.

Trusted matters here, and it is the point rather than a detail. The planner may read those contents,
compare them, and act on what they say, which is what makes a reference useful and what a quarantined
read deliberately withholds. So reference a file when you want it worked on, and be as careful about
it as you would be answering yes to a directory: content you have not read is content you are
vouching for.

A directory is somewhere to type through rather than a file to read, so naming one includes nothing.
References cannot leave the workspace: `..` and an absolute path are refused rather than resolved,
exactly as they are everywhere else.

## Opening another directory

`/add-dir ~/notes` opens a second directory for the rest of the session. Two things have to happen
for that to be useful, and it does both: the directory becomes **reachable**, since an absolute path
is otherwise refused whatever the trust map says, and it is recorded as **trusted**, which is what
stops every write there asking. Either alone is no use, one leaving a rule about files nothing can
open and the other leaving a directory that prompts on every edit.

Inside it, the rules are the working directory's own. `..` cannot climb out, a symlink leaving it is
refused, and a relative path still always means the project, so no file ends up with two spellings.

It lasts for the session, like every other answer here, and `--resume` carries it. `/clear` closes it
again, since that begins a session of its own. A directory already inside the project is refused,
since it is reachable by its relative path already.

## A path outside the working directory is a separate rule

Trust rules come in two kinds, and neither says anything about the other. A **relative** rule is
a path under the working directory. An **absolute** rule names a directory you added by its own
name, which is what `/add-dir` records.

They are kept apart because the same relative path can exist in both places. A rule about `src`
would otherwise decide `src/main.rs` in a directory you added as well as in the project, and the
two are different files.

The separation is load-bearing rather than tidy. Rules are matched by prefix, and the working
directory's own rule is the empty prefix that covers everything beneath it. Treat `/` as that same
empty prefix and trusting one added directory would silently vouch for every file in the project,
which is the laundering the rest of this document exists to prevent.

## Your own directory is trusted, because it is yours

The table above is about the working directory. `~/.bua` is different: it holds your history,
your sessions, your standing instructions, and your skills, and everything in it is something you
put there. It is read as trusted for that reason, on the same footing as the configuration that
already picks your model and your endpoint.

This is not trust assumed from silence, which is the thing the rest of this document refuses.
An empty directory offers nothing at all. **Putting a file there is the grant**, exactly as
answering yes to a working directory is.

The consequence, worth stating rather than burying: a skill you downloaded into `~/.bua/skills`
is trusted as far as a config file you pasted is. Read one before you install it. See
[skills.md](skills.md).

Files in a *project* get no such standing. A project's `AGENTS.md` and `.bua/skills` are read
through the table above, so they load when you vouched for the directory and are left out when
you did not.
