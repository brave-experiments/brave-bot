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
