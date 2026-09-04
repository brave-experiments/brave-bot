---
id: DROP
title: Dropping a file on the window
status: normative
governs:
  - crates/tui/src/dropped.rs
  - crates/tui/src/app.rs
---

## Scope

What happens when a person drags a file onto the terminal, and on what footing it enters the turn.
What the box does with the marker afterwards is [terminal-input.md](terminal-input.md).

Three gestures put content into a turn on the user's own footing, and each has its own spec:
[naming-files.md](naming-files.md) for `@` in a prompt, [pasting.md](pasting.md) for Ctrl-V, and
[dropping.md](dropping.md) for a file dragged onto the window.

## Why a dropped file is trusted

Because a person picked that one file and let the line go. It enters the turn on the footing of the
words typed beside it: the path came from a gesture rather than from anything a model said, and
sending the line is the grant. Nothing inspects the contents and nothing could.

What makes reaching outside the workspace sound is not where the file sits but where its path came
from. A dropped file's path is fixed before the turn starts, and nothing a model says or a file
contains can put one there.

## Clauses

<a id="DROP-1"></a>
### DROP-1: only a file a person dropped

Never a path a model proposed, never one read out of a file, never one a processor produced. The
justification cannot be checked from the bytes, so it lives at the call site: today that is the
terminal's drop handling and nothing else.

`verified-by: bravebot_tui::drop::dropping_an_image_puts_a_marker_in_the_line`
`verified-by: bravebot_agent::workspace::an_untrusted_path_is_not_read_as_a_drop`

<a id="DROP-2"></a>
### DROP-2: a drop makes that file trusted

The rule recorded is for the file itself, so its contents can be read and it can be edited for the
rest of the session. A rule on a file is more specific than any rule on the tree around it, so a
dropped file is trusted even inside a directory marked untrusted.

`verified-by: bravebot_agent::turn::attaching_a_file_vouches_for_it_the_way_naming_one_does`

<a id="DROP-3"></a>
### DROP-3: a drop makes that file reachable, wherever on the disk it is

A dropped file, and only a dropped file, may name a path outside the working directory. Whether it
is carried as bytes or read as text makes no difference, since the same gesture produced both.

Both grants are for the one file. Nothing else in the directory it came from becomes trusted or
reachable, and reading, writing, editing, listing and searching stay confined exactly as they
were.

**Why.** A drop can come from anywhere on the disk and usually does, because the place someone
drags a file from is rarely inside the project they are working on. Confining a drop to the
workspace would refuse the ordinary case. What makes reaching out sound is not where the file sits
but that a person's gesture put its path there.

`verified-by: bravebot_tui::drop::a_drop_from_outside_the_workspace_is_attached_all_the_same`
`verified-by: bravebot_tui::drop::a_text_file_from_outside_the_workspace_is_dropped_all_the_same`
`verified-by: bravebot_tui::drop::the_name_handed_to_the_task_is_relative_to_the_workspace`
`verified-by: bravebot_agent::workspace::a_dropped_text_file_may_come_from_outside_the_workspace`
`verified-by: bravebot_agent::turn::a_dropped_text_file_from_outside_the_workspace_becomes_context`
`verified-by: bravebot_agent::turn::dropping_a_text_file_does_not_reach_anything_beside_it`

<a id="DROP-4"></a>
### DROP-4: a recognised type is carried, and an unrecognised one is only a path

Images and PDFs are carried as bytes, so the model looks at them. A text file becomes context, its
contents entering the turn as trusted input. A type nothing here takes has its path written into the line instead, which is
what dropping a file did before any of this existed. Extensions are recognised whatever their case.

`verified-by: bravebot_tui::drop::a_dropped_text_file_is_context_rather_than_an_attachment`
`verified-by: bravebot_tui::drop::dropping_an_unsupported_type_writes_out_the_path`
`verified-by: bravebot_tui::dropped::an_unsupported_type_is_a_drop_that_attaches_nothing`
`verified-by: bravebot_tui::dropped::an_unsupported_file_beside_a_supported_one_leaves_it_attachable`
`verified-by: bravebot_tui::dropped::an_extension_is_recognised_whatever_its_case`
`verified-by: bravebot_tui::dropped::the_recognised_types_are_the_ones_claude_code_takes`

<a id="DROP-5"></a>
### DROP-5: dropping a directory attaches nothing

A directory is somewhere to type through rather than a file to read, so naming one includes nothing.

`verified-by: bravebot_tui::drop::dropping_a_directory_attaches_nothing`

<a id="DROP-6"></a>
### DROP-6: each dropped file gets its own marker, and deleting one takes it off

`[Image #1]`, numbered so a second drop is distinguishable from the first, each keeping its place in
a mixed drop. Deleting the marker is the only way to change your mind, and sending the line clears
what was attached to it.

`verified-by: bravebot_tui::drop::several_files_dropped_together_each_get_a_marker`
`verified-by: bravebot_tui::drop::a_second_drop_gets_its_own_number`
`verified-by: bravebot_tui::drop::a_mixed_drop_keeps_each_in_its_place`
`verified-by: bravebot_tui::drop::deleting_the_marker_takes_the_attachment_off`
`verified-by: bravebot_tui::drop::sending_a_line_clears_what_was_attached_to_it`
`verified-by: bravebot_tui::drop::a_drop_leaves_room_after_itself`

<a id="DROP-7"></a>
### DROP-7: a line is a drop only when every word of it is a path that exists

Terminals deliver a drop as text, so it has to be told from typing. A plain, quoted, backslash
escaped or `file://` path counts, several at once count, and a percent sign in a name survives. One
word of prose, a path naming nothing, an unterminated quote, an empty paste, or more than one line
makes it a paste instead.

**Why.** Guessing wrong in the permissive direction would attach a file because somebody mentioned
its name, which is a path nobody's gesture put there.

`verified-by: bravebot_tui::dropped::a_plain_path_is_a_drop`
`verified-by: bravebot_tui::dropped::a_quoted_path_is_unquoted`
`verified-by: bravebot_tui::dropped::a_backslash_escaped_path_is_unescaped`
`verified-by: bravebot_tui::dropped::a_file_uri_becomes_a_path`
`verified-by: bravebot_tui::dropped::a_literal_percent_in_a_name_survives`
`verified-by: bravebot_tui::dropped::several_files_dropped_at_once_are_all_taken`
`verified-by: bravebot_tui::dropped::prose_mentioning_a_real_file_is_not_a_drop`
`verified-by: bravebot_tui::dropped::a_path_that_names_nothing_is_not_a_drop`
`verified-by: bravebot_tui::dropped::one_word_of_prose_is_enough_to_make_it_a_paste`
`verified-by: bravebot_tui::dropped::a_multi_line_paste_is_never_a_drop`
`verified-by: bravebot_tui::dropped::an_unterminated_quote_is_not_a_drop`
`verified-by: bravebot_tui::dropped::an_empty_paste_is_not_a_drop`
`verified-by: bravebot_tui::drop::pasting_prose_about_a_real_file_is_still_prose`

<a id="DROP-8"></a>
### DROP-8: a file dropped onto a line queued mid-turn is named, not carried

A prompt sent while a turn is running goes into that running turn rather than waiting for the next
one ([terminal-input.md](terminal-input.md)), and a turn already running cannot be handed a file: it
precommitted its routing and fixed the shape of its context before it read anything, so there is no
trusted slot for one to arrive in. What the planner is given instead is the file's
**name**, in place of the marker, settled at the moment the line was sent. A pasted picture has no
such recourse and resolves to a sentence saying a picture was pasted and cannot be shown.

Only what talks to the model sees this. The box, the transcript and the history keep the marker,
exactly as a folded paste keeps its own marker, because that is what was on the person's screen.
See [pasting.md](pasting.md).

**Why.** Dropping a file mid-turn is telling the agent which file to look at, and a name is enough
for that: the planner reads it and goes to the file through the same gate it reads any other file
through, with the trust map deciding as it always does. The two alternatives are worse. Leaving the
marker in place sends `[Image #1]`, which stands for nothing the planner can use. Admitting the file
would mean minting a trusted path in the middle of a turn from a string that crossed a thread
boundary after untrusted content had already been observed, which is the property
[routing.md](routing.md) exists to hold.

A prompt that is still waiting when the turn ends is not affected: it becomes a turn of its own, and
as its own turn it carries its files and pictures the way any prompt does.

`verified-by: bravebot_tui::drop::a_file_dropped_into_a_queued_line_is_named_to_the_planner`
`verified-by: bravebot_tui::drop::resolving_a_queued_line_does_not_rewrite_what_the_person_sees`

## Known costs

- **A screenshot somebody sent you is content you have not read and are vouching for.** It goes
  into the turn as trusted input, on the strength of the gesture alone. Be as careful about a drop
  as about answering yes to a directory.
