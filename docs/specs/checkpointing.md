---
id: CHECKPOINT
title: Checkpoints and rewinding
status: proposed
governs:
  - crates/agent/src/tools.rs
  - crates/tui/src/app.rs
  - crates/tui/src/sessions.rs
---

## Scope

Undoing what a turn did: the contents kept before this agent changed a file, the conversation
position those contents belong to, and what restoring one is allowed to put back. What is kept
between runs so a session can be picked up again is [sessions.md](sessions.md), and a checkpoint
deliberately stays out of it. When a person is asked before an effect, and what their answer
grants, is [prompting.md](prompting.md). What makes a line a command at all is
[commands.md](commands.md), which owns the surface rather than what any one command does.

**Part of this specifies work that does not exist yet.** Keeping a checkpoint and listing one are
built and pinned by tests. Restoring one is not: every clause under Restoring, and the clause about
what a person approves, says `verified-by: none` and describes what the work will have to do. Each
gains the tests that pin it in the commit that implements it.

Naming a checkpoint by hand is deliberately not specified here. A checkpoint is made for every turn
that changes a file, so the point a person wants is already kept; a name would be a second way to
say what the row says already.

## Where a checkpoint lives

<a id="CHECKPOINT-1"></a>
### CHECKPOINT-1: a checkpoint belongs to a session, not to a directory

Checkpoints are listed and restored inside the session that made them. Two sessions working in one
directory keep separate histories, so rewinding either never moves the other, and neither offers
the other's checkpoints to return to.

**Why.** A directory is where work happens, and a session is the conversation that did it.
Restoring is a statement about a conversation. A checkpoint made while one session was reworking
authentication says nothing about another session editing the database in the same checkout, and
offering it there would invite a person to undo work they were never doing.

`verified-by: bravebot_tui::checkpoints::two_sessions_in_one_directory_do_not_share_checkpoints`

<a id="CHECKPOINT-2"></a>
### CHECKPOINT-2: a checkpoint is not part of the session record

A checkpoint is never a field of the record, is never written by what writes the record, and
deleting every checkpoint leaves a session that still resumes.

**Why.** The record is read back into a later turn's context on a resume, so nothing may be written
into it that the planner could not have held. A checkpoint holds the prior contents of files, and a
file this agent changed may be one nobody vouched for, so those contents cannot live anywhere a
resume reads. Keeping the two apart is what makes that hold by construction, rather than by
remembering to filter on the way out.

`verified-by: bravebot_tui::checkpoints::checkpoints_do_not_live_under_the_session_records`

<a id="CHECKPOINT-3"></a>
### CHECKPOINT-3: nothing kept for a checkpoint ever reaches a turn

Contents kept for a checkpoint are written to disk and read back only in order to restore them.
They never enter a request, are never summarised into one, and are never put in front of the
planner in any form.

**Why.** This is the largest store of possibly untrusted content the program keeps. Content from it
reaching a request would be the laundering route the whole design exists to close, and it would
arrive labelled as history rather than as something a turn had just read, which is the shape least
likely to be questioned.

`verified-by: none`

<a id="CHECKPOINT-4"></a>
### CHECKPOINT-4: what a session keeps is bounded, and the oldest goes first

Checkpoint storage is limited, by count or by total size. Passing the limit drops the oldest
checkpoints rather than refusing to make a new one. The prompts a person typed are bounded the same
way, for the same reason.

**Why.** A file's contents are far bulkier than a line of transcript, so a long session in a large
repository would grow without limit on a machine used for years. Dropping the oldest is the right
direction: a rewind reaches for something recent, because a turn that went wrong is noticed within
a few turns or not at all. Refusing to make a new checkpoint would instead withdraw the protection
exactly when a session has been running long enough to need it.

`verified-by: bravebot_tui::checkpoints::passing_the_bound_drops_the_oldest_rather_than_refusing_a_new_one`

<a id="CHECKPOINT-5"></a>
### CHECKPOINT-5: a checkpoint is readable only by the person who made it

Checkpoint files are created readable and writable by their owner alone, whatever the process
umask would otherwise give them, and the directory holding them is created the same way.

**Why.** A checkpoint holds the contents of source files, which on a private project is the
project, along with whatever secret happened to be sitting in a file this agent edited. The scratch
file a prompt is composed in is already written this way, and it holds one line where a checkpoint
holds a great deal more. A permissive umask is the ordinary case rather than the unusual one, so a
mode left to it is a mode chosen by nobody.

`verified-by: bravebot_tui::checkpoints::a_checkpoint_is_written_readable_by_nobody_else`

## What a checkpoint holds

<a id="CHECKPOINT-6"></a>
### CHECKPOINT-6: a checkpoint captures only what this agent changed

The files in a checkpoint are the ones this agent wrote or edited. The working tree is not
captured, and a file the person changed themselves is never in a checkpoint unless this agent
changed it too.

**Why.** Capturing a tree is not viable in a large repository, and it would make the program
responsible for restoring work it never touched. The files it changed are also the only ones whose
prior contents it was ever in a position to keep, because it held them immediately before writing
over them.

`verified-by: bravebot_agent::turn::overwriting_a_file_announces_what_it_held_first`
`verified-by: bravebot_agent::turn::editing_a_file_announces_what_it_held_first`
`verified-by: bravebot_agent::turn::a_refused_write_leaves_nothing_to_return_to`

<a id="CHECKPOINT-7"></a>
### CHECKPOINT-7: a captured file is kept whole, never as a difference

What is stored is the file's prior contents in full. A checkpoint never stores a difference against
the current file, and a restore is never reconstructed by applying one.

**Why.** Applying a difference means locating the passage it belongs to, and locating a passage
means comparing text, which is a decision. A file this agent changed may be one nobody vouched for,
and a decision taken from bytes an attacker may have written is what this system refuses
everywhere else: the same reasoning is why an edit is refused on a file nobody vouched for and a
whole-file write is the route instead. Whole contents are carried and handed to a write with
nothing compared along the way, so the bulkier representation is the only one that stays inside the
rule. Growth is answered by the bound above, not by a cleverer encoding.

`verified-by: bravebot_tui::checkpoints::what_a_turn_changed_is_written_and_read_back`

<a id="CHECKPOINT-8"></a>
### CHECKPOINT-8: a path that held no file is captured as holding none

A checkpoint taken before this agent creates a file records that the path was empty. Restoring it
removes the file rather than leaving one with nothing in it.

**Why.** Otherwise creating a file is the single change a rewind cannot undo, and what it leaves
behind is a file the person never had. Returning somewhere and finding an artefact of the visit is
worse than not returning, because nothing says the state is not the one that was asked for.

`verified-by: bravebot_agent::turn::creating_a_file_announces_that_the_path_held_nothing`
`verified-by: bravebot_tui::checkpoints::a_path_that_held_no_file_is_captured_as_holding_none`

## Restoring

<a id="CHECKPOINT-9"></a>
### CHECKPOINT-9: restoring a file is an ordinary write

A restore passes the gates every other write passes. It needs the capability, it needs a person's
endorsement, and it cannot land outside the workspace. Being an undo earns no shortcut.

**Why.** The bytes going back may be untrusted, and nobody has looked at them since they were
captured. A restore that wrote without asking would be the one write in the program putting
unreviewed content on disk, and it would do it at the moment a person is least likely to be reading
closely, because putting things back sounds like the safe direction.

`verified-by: none`

<a id="CHECKPOINT-10"></a>
### CHECKPOINT-10: restored content carries the label it was captured with

Content comes back labelled as it was labelled when it was kept. Nothing becomes trusted for having
been on disk earlier, and no checkpoint raises a label.

**Why.** Labels only ever degrade, and age is not provenance. A file that was untrusted when this
agent read it is untrusted when it goes back, and a store that quietly improved a label on the way
through would be laundering with a delay on it.

`verified-by: none`

<a id="CHECKPOINT-11"></a>
### CHECKPOINT-11: a rewind restores standing permissions and nothing else

Rewinding past a turn restores what a resume would restore, meaning the trust map and the list of
commands a person said to stop asking about, and nothing further. A single-use endorsement is never
revived: an approval given for one act stays spent, and a restore needing one asks for it again.

**Why.** A standing permission is a decision about the future, and the person rewinding is the
person who made it. An endorsement is a decision about one act that has already happened, and
reviving one would approve something nobody looked at. A rewind is a good moment for that mistake,
since the turns being discarded are exactly the ones whose approvals are freshest.

`verified-by: none`

<a id="CHECKPOINT-12"></a>
### CHECKPOINT-12: a rewind restores a coherent state, or it refuses

Where a discarded turn did two things that only mean anything together, a rewind undoes both or
neither. Opening another directory is the case that exists today: it makes a tree reachable and it
records the rule vouching for that tree. A rewind past it returning one half without the other
would leave either a rule about files nothing can open, or a reachable tree nothing vouched for.

**Why.** A half-undone state is worse than no undo, because the person believes they are somewhere
they have never been, and the belief is what they act on next. Refusing is the honest answer when
both halves cannot be returned, and it is a failure the person can see rather than one they find
later.

`verified-by: none`

## Git

<a id="CHECKPOINT-13"></a>
### CHECKPOINT-13: checkpointing never writes the user's git state

Nothing in making or restoring a checkpoint changes a repository: not the index, not the stash, not
a branch, not the working tree by way of git. Reading is allowed, and is done by reading the files
rather than by running git, as the branch shown on a session listing already is.

**Why.** The case worth protecting is a tree holding the person's own uncommitted work, and every
git operation able to undo this agent's changes takes theirs along with it. A checkpoint is session
recovery and a commit is project history: reaching for one to do the other spends the person's
version control on something it did not agree to, and the tree that most needs a rewind is the one
where that costs most.

`verified-by: none`

## What a person sees

<a id="CHECKPOINT-14"></a>
### CHECKPOINT-14: a checkpoint is made for each turn that changes a file

A turn leaves one checkpoint behind, holding what it wrote over, and a turn that changed no file
leaves none. Nobody has to ask for it.

**Why.** A rewind is wanted for the turn that just went wrong, so the useful checkpoint is the one
nobody thought to ask for: the program has to take it unasked or it will not be there. A turn that
changed nothing is left out because a point that returns to where you already are is one more row
between a person and the row they want.

`verified-by: bravebot_tui::checkpoints::a_turn_that_wrote_nothing_leaves_no_checkpoint`
`verified-by: bravebot_tui::checkpoints::two_writes_to_one_path_add_up_to_one_row`

<a id="CHECKPOINT-15"></a>
### CHECKPOINT-15: `/checkpoints` lists what this session can return to

One row each, newest first. A row says which checkpoint it is, what the checkpoint is, and how long
ago it was made. It shows no file contents, and it never says which turn. Checkpoints belonging to
other sessions are not listed.

Asking again replaces the list rather than drawing a second copy below the first.

**Why replacing.** Asking twice is how a person checks whether the turn they just had left a point
behind, so it is the ordinary way to use this rather than a mistake. A report that stacks up answers
the same question three times over and buries the transcript it was asked about, and the rows are
the same rows: nothing is lost by dropping the older copy.

What a row says about the change, and which of those words are the model's, is the subject of
its own clause below.

**Why.** A list is scrolled past, and putting the contents of files into it would put a great deal
of unread content on screen for a gesture that decides nothing. The decision is the restore, and
that is where what would change is shown.

**Why its own number rather than the turn.** A turn that changes no file leaves no checkpoint, so
turns skip: a list numbered by turn reads 1, 2, 4 and invites a person to wonder what happened to
3. A checkpoint's own number is also settled when it is made and does not move as more arrive,
which is what makes it worth quoting back: a restore has to be asked for by something, and a number
that means a different point tomorrow is not it.

`verified-by: bravebot_tui::checkpoints::a_list_puts_the_most_recent_checkpoint_first`
`verified-by: bravebot_tui::checkpoints::a_row_keeps_its_own_number_however_many_arrive`
`verified-by: bravebot_tui::checkpoints::a_row_says_which_checkpoint_and_never_which_turn`
`verified-by: bravebot_tui::checkpoints::each_checkpoint_is_one_row_and_no_more`
`verified-by: bravebot_tui::checkpoints::asking_twice_leaves_one_list_in_the_transcript`
`verified-by: bravebot_tui::checkpoints::replacing_the_list_leaves_the_rest_of_the_transcript_alone`
`verified-by: bravebot_tui::checkpoints::a_list_shows_no_file_contents`
`verified-by: bravebot_tui::checkpoints::a_session_with_nothing_to_return_to_draws_no_rows`

<a id="CHECKPOINT-16"></a>
### CHECKPOINT-16: a restore is approved from the changes themselves, not a summary

Before anything is written back, the person is shown what would change, as the differences that
would be applied rather than a count of files or a sentence describing them. What is shown is drawn
inside a margin it cannot forge, since content coming back may be untrusted. Where the changes
cannot be shown legibly the review says so, rather than presenting something it cannot stand
behind.

**Why.** A prompt saying twelve files will be restored asks a person to approve what they have not
seen. A restore is the write most likely to be waved through, because it is framed as putting
things back, and the framing is doing the work rather than the review. Every other write in the
program is approved from a diff, and this is the one where the difference between the two states is
the entire question.

`verified-by: none`

<a id="CHECKPOINT-17"></a>
### CHECKPOINT-17: a row says what the checkpoint is, and marks the words the model wrote

A row says what that turn did, in the model's account of it, cut to a line and beginning at what
was done: an opening flourish and the words saying who did it are dropped, so a row reads "created
notes.md" rather than "Done! I've created notes.md". Nothing else is rewritten. Where there is no
account at all, this program's own record of the files: what was written, what was done to each,
and how many lines went in and out, with a created file saying so rather than counting every line
as added.

A summary the model wrote is drawn inside a margin the renderer paints, on every drawn row, exactly
as the model's output is marked anywhere else. A record this program made carries no margin,
because marking it would say the opposite of what is true and teach a reader to ignore the bar.

The words the model wrote are taken from the reply the person was already shown, so a row cannot
say something the transcript never said.

**Why.** The model's account reads far better than a file name and a pair of counts, and a list
nobody can read is a list nobody uses. It begins at the verb because that is the word somebody
scanning a list is looking for, and because every reply opens the same way: a column of rows all
starting "Done! I've" spends its width saying nothing. What makes it safe to show is not that it is reliable: it is
untrusted text, free to describe a change as something gentler than it was, and this row is what a
person picks a restore from. What makes it safe is that it is marked, that the counts beside it are
this program's own, and that the restore itself is approved from the actual differences rather than
from any sentence about them. A summary can mislead about what a turn did; it cannot make a person
approve a change they have not seen.

**Not what the turn was asked.** That is trustworthy and answers a different question: a turn can
be asked for one thing and do another, and a row a restore is chosen from has to be about what
happened.

`verified-by: bravebot_tui::checkpoints::a_summary_the_model_wrote_is_reported_as_untrusted`
`verified-by: bravebot_tui::checkpoints::a_row_says_what_was_done_and_how_much`
`verified-by: bravebot_tui::checkpoints::a_created_file_says_so_rather_than_counting_its_lines`
`verified-by: bravebot_tui::checkpoints::a_turn_that_changed_several_files_names_one_and_counts_the_rest`
`verified-by: bravebot_tui::checkpoints::a_reply_is_cut_at_its_first_sentence`
`verified-by: bravebot_tui::checkpoints::a_row_starts_at_what_was_done`
`verified-by: bravebot_tui::checkpoints::a_first_person_opening_is_dropped`
`verified-by: bravebot_tui::checkpoints::a_reply_already_starting_at_the_verb_is_left_alone`
`verified-by: bravebot_tui::checkpoints::a_reply_spanning_lines_becomes_one_line`
`verified-by: bravebot_tui::render::a_model_written_checkpoint_summary_is_drawn_behind_a_margin`
`verified-by: bravebot_tui::render::a_checkpoint_summary_cannot_paint_its_own_margin`
`verified-by: bravebot_tui::render::a_row_the_driver_wrote_carries_no_margin`
