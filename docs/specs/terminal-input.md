---
id: INPUT
title: The input box
status: normative
governs:
  - crates/tui/src/app.rs
  - crates/tui/src/state.rs
  - crates/tui/src/wrap.rs
  - crates/tui/src/editor.rs
---

## Scope

What the user types into: how the box behaves, which keys do what, and where a terminal's own
limits show through. What is drawn back is [terminal-transcript.md](terminal-transcript.md), and
pasting is [pasting.md](pasting.md).

## Clauses

<a id="INPUT-1"></a>
### INPUT-1: the box grows with the text, up to a cap

The cap is ten rows, enough for a substantial paragraph while the transcript keeps the majority of
a standard terminal. Beyond it the box scrolls to the cursor rather than growing further, and it
keeps growing while a turn runs. A long list of indicators leaves room for the box and the
transcript, and the box stays beneath the list.

**Why.** The line being composed is the one thing a person must always be able to see.

`verified-by: bravebot_tui::render::the_input_box_grows_with_the_text`
`verified-by: bravebot_tui::render::the_input_box_stops_growing_at_the_cap`
`verified-by: bravebot_tui::render::a_very_long_input_scrolls_to_the_cursor`
`verified-by: bravebot_tui::render::the_box_grows_mid_turn_too`
`verified-by: bravebot_tui::render::a_long_list_leaves_room_for_the_box_and_the_transcript`
`verified-by: bravebot_tui::render::the_box_stays_beneath_the_list`


<a id="INPUT-2"></a>
### INPUT-2: Shift-Enter starts a line, Enter sends

Ctrl-J does the same thing everywhere and needs no terminal configuration, because most terminals
send the same byte for Enter whichever modifier was held. Both work while a turn runs and in shell
mode. A newline lands at the caret, does not arm shell mode, and Enter still sends a paragraph
written this way. Enter on an empty line does nothing.

`verified-by: bravebot_tui::app::shift_enter_starts_a_line_instead_of_sending`
`verified-by: bravebot_tui::app::ctrl_j_starts_a_line_too`
`verified-by: bravebot_tui::app::ctrl_j_is_not_swallowed_while_a_turn_runs`
`verified-by: bravebot_tui::app::shift_enter_works_in_shell_mode`
`verified-by: bravebot_tui::app::shift_enter_works_while_a_turn_runs`
`verified-by: bravebot_tui::app::a_newline_lands_at_the_caret`
`verified-by: bravebot_tui::app::a_newline_does_not_arm_shell_mode`
`verified-by: bravebot_tui::app::enter_still_sends_a_paragraph_written_with_shift_enter`
`verified-by: bravebot_tui::app::enter_submits_the_prompt`
`verified-by: bravebot_tui::app::enter_on_empty_input_does_nothing`


<a id="INPUT-3"></a>
### INPUT-3: a marker is deletable, and deleting it takes the thing off

This holds for a folded paste, a pasted picture and a dropped file alike. It
is why a marker exists rather than a list the user cannot edit.

The row beneath the box goes with the marker. What is drawn there is what the line in the box
carries, so a file whose marker has been rubbed out is drawn nowhere, and one whose marker is still
there is drawn whether or not a turn is running.

**Why.** The turn was always built from the markers the line still held, so a deleted one already
sent nothing. What lingered was the row, which is the only place a person can see whether rubbing
the marker out worked: left drawn, it says a file is going that is not.

`verified-by: bravebot_tui::drop::deleting_the_marker_takes_the_attachment_off`
`verified-by: bravebot_tui::drop::several_files_dropped_together_each_get_a_marker`
`verified-by: bravebot_tui::drop::sending_a_line_clears_what_was_attached_to_it`
`verified-by: bravebot_tui::render::an_attached_file_is_named_under_the_box`
`verified-by: bravebot_tui::render::deleting_the_marker_takes_the_row_out_from_under_the_box`


<a id="INPUT-4"></a>
### INPUT-4: the keys that stop, and the one that also leaves

Escape discards a half-typed prompt, and does nothing at all on a line with nothing on it. It
never ends the session.

**Ctrl-C stops the nearest thing there is to stop, and leaves when there is nothing left.** It is
read against what is happening, in this order:

| What is happening | What Ctrl-C does |
|---|---|
| a turn in flight, or a command running | stops it, and the session stays where it was |
| nothing running, a line in the box | takes the line, and offers the way out |
| nothing running, an empty box | ends the session |

Escape only ever stops, and never leaves. A summary is the one exception to the table: it is a
single request with no round for a stop to land between, so nothing there can stop it and Ctrl-C
leaves once it comes back.

Taking the line says so, on the line beneath the box, and says which key ends the session. The
offer lives for exactly one press, since it answers the press just made and the next press is the
answer to it. Nothing is said where the box was already empty: that press leaves, and a press that
leaves is not one to explain.

**Stopping is silent, and the prompt comes back.** Neither key says that it is stopping. A reply
still arriving stops arriving, the prompt that was sent returns to the box for editing, and that
is the whole of the answer. There is nothing to wait through and so nothing to report waiting on.

The prompt stays sent, marked stopped, where either of two things is true: the turn had already
done something that is on the screen, or there are prompts waiting behind it. Both mean there is
an order to keep, and a line put back in the box would be out of it.

**Nothing waits out work that is only being waited on.** A reply stops whether or not it has begun
arriving, and whether it is the planner's or a processor's; a running command is killed; a pause
between retries is abandoned; and nothing new is sent once a stop has landed. What still finishes
is a tool call already running, because stopping one part way could leave a file half written.

A request being waited on is walked away from rather than interrupted, since a read in progress
cannot be interrupted. The socket is left to be closed when the far end finishes or the connection
times out, which is sound because reading a reply applies nothing and decides nothing.

**Why.** It used to say "cancelling…" and go on streaming the reply to the end, because a stop was
only noticed between rounds. So the key that was supposed to stop the answer left the answer
running and put a progress report on the screen about a key press, and the longer the reply the
longer somebody waited for the thing they had already stopped.

**Why.** The press somebody makes while an answer is going wrong in front of them is asking for
the answer to stop, not for the session to end, and answering it by leaving takes the transcript
and everything else with it. Ctrl-C is also how a person leaves a terminal program, which is the
other half: it leaves from an empty box, so both requests have a key.

This is not the arrangement where a key pressed twice means two different things by accident. Each
press has something of its own to answer and the state says which, so no press is one that
silently did another one's job. What makes the ladder safe to walk is that each rung is visible:
the turn stopping is on the screen, and the line going says what the next press will do.

Escape used to leave as well, once the line was empty. That made every press a question of what
was in the box: the key for abandoning a thought ended the session as soon as the thought was
short enough, and pressing it twice in a row meant two different things, the second of which was
the exit. One way out, and it is the one people already reach for.

`verified-by: bravebot_tui::app::escape_clears_a_typed_line_without_quitting`
`verified-by: bravebot_tui::app::escape_on_an_empty_line_does_not_quit`
`verified-by: bravebot_tui::app::escape_twice_clears_and_stays`
`verified-by: bravebot_tui::app::ctrl_c_quits`
`verified-by: bravebot_tui::app::ctrl_c_stops_a_turn_rather_than_leaving`
`verified-by: bravebot_tui::app::ctrl_c_clears_the_line_before_it_leaves`
`verified-by: bravebot_tui::app::ctrl_c_leaves_once_there_is_nothing_left_to_stop`
`verified-by: bravebot_tui::app::the_way_out_is_offered_only_where_a_line_was_taken`
`verified-by: bravebot_tui::app::the_way_out_stops_being_offered_at_the_next_press`
`verified-by: bravebot_tui::render::the_way_out_is_offered_where_the_line_went`
`verified-by: bravebot_tui::app::escape_only_stops_and_ctrl_c_is_read_against_what_is_happening`
`verified-by: bravebot_aichat::client::a_stopped_stream_stops_before_the_reply_is_over`
`verified-by: bravebot_aichat::client::a_stream_stopped_before_it_starts_reports_nothing`
`verified-by: bravebot_aichat::client::a_stop_does_not_wait_out_the_pause_between_attempts`
`verified-by: bravebot_aichat::client::a_stop_does_not_wait_for_the_model_to_start_writing`
`verified-by: bravebot_tui::state::cancelling_before_anything_happens_still_un_sends_the_prompt`
`verified-by: bravebot_tui::app::a_key_that_would_stop_a_turn_is_answered_during_a_summary`
`verified-by: bravebot_tui::app::escape_stops_the_turn_without_ending_the_session`
`verified-by: bravebot_tui::app::ctrl_g_asks_for_the_editor`


<a id="INPUT-5"></a>
### INPUT-5: where a chord cannot reach the process, the fallback is documented rather than silent

Shift-Enter needs a terminal that reports the modifier (Ghostty, Kitty, WezTerm) or one configured
to send a newline; Ctrl-J is the fallback that always works (INPUT-2). Command-V never reaches the
process and can carry only text, so Ctrl-V is the key for a picture, and which key
carries a picture is said once per session.

**Why.** A chord that silently does nothing reads as a broken feature.

`verified-by: bravebot_tui::app::which_key_carries_a_picture_is_said_once_per_session`
`verified-by: bravebot_tui::app::a_paste_clears_the_hint_that_prompted_it`


<a id="INPUT-6"></a>
### INPUT-6: a marker is deleted whole, in one press

Backspace and Delete each take the whole of the marker the caret covers, and Backspace takes the
whole of one it sits just after, whether it stands for a folded paste, a pasted picture or a
dropped file. A covered marker goes before the character in front of it, because it is the thing
the caret is on. Only a marker the box wrote goes this way: square brackets the user typed are
deleted a character at a time, as everything they typed is.

**Why.** A marker is one thing on the screen and one thing to the person looking at it. Taking a
character off the end leaves text that still reads as an attachment standing over something no
longer attached, and the only way to find that out is to keep pressing.

`verified-by: bravebot_tui::state::one_backspace_takes_the_whole_marker`
`verified-by: bravebot_tui::state::backspace_on_a_covered_marker_takes_the_marker`
`verified-by: bravebot_tui::state::one_backspace_takes_the_whole_folded_paste`
`verified-by: bravebot_tui::state::delete_forward_takes_the_whole_marker`
`verified-by: bravebot_tui::state::text_that_merely_looks_like_a_marker_is_deleted_one_character_at_a_time`
`verified-by: bravebot_tui::drop::one_backspace_takes_the_whole_marker`


<a id="INPUT-7"></a>
### INPUT-7: the caret steps over a marker whole, and never rests inside one

One press of Left or Right crosses a marker in either direction, and there is no position within
one for the caret to stop at.

**Why.** A caret between two halves of a picture is in a place the person cannot see, and whatever
they type next lands there. Counting out the characters a marker happens to be spelled with is a
dozen presses to cross what reads as a single word.

`verified-by: bravebot_tui::state::the_caret_steps_over_a_marker_whole`
`verified-by: bravebot_tui::state::the_caret_cannot_come_to_rest_inside_a_marker`


<a id="INPUT-8"></a>
### INPUT-8: the caret is drawn over the whole marker it is on

Every cell of the marker is covered, including the part of one the box wrapped onto the next row.

**Why.** The caret says what the next press acts on. A block over the opening bracket alone says
the next press takes a bracket, which is the thing that no longer happens.

`verified-by: bravebot_tui::render::the_caret_covers_a_whole_marker`
`verified-by: bravebot_tui::render::a_marker_the_wrap_split_is_covered_on_both_rows`

<a id="INPUT-9"></a>
### INPUT-9: the box behaves the same whether or not a turn is running

Typing, editing, pasting, dropping a file, completing, walking back through earlier prompts and
scrolling the transcript all do while a turn is in flight exactly what they do at rest. What a
running turn refuses is **sending**, and nothing else.

**Why.** The box took nothing at all mid-turn once, and it was opened up a piece at a time:
characters, then editing, then pasting. Walking the history was left behind, so a person could
compose a new prompt during a turn but could not reach the one they had just sent, which is the
one they want most when a turn is going wrong in front of them. The keys reached no arm and did
nothing at all, not even the scrolling they fall through to at rest.

A difference between the two has to be a difference about sending. Taking back a prompt that has
been sent and has not gone yet (INPUT-18) is one; anything else is drift, and this one was silent,
so the two paths walk one list of keys rather than a list each.

`verified-by: bravebot_tui::app::the_navigation_keys_do_the_same_thing_whether_or_not_a_turn_is_running`
`verified-by: bravebot_tui::app::up_recalls_a_previous_prompt_while_a_turn_is_running`
`verified-by: bravebot_tui::state::recall_works_while_a_turn_is_running`
`verified-by: bravebot_tui::state::a_recalled_prompt_still_cannot_be_sent_while_a_turn_is_running`
`verified-by: bravebot_tui::app::a_long_paste_folds_while_a_turn_is_running`
`verified-by: bravebot_tui::app::a_file_dropped_while_a_turn_is_running_is_attached`
`verified-by: bravebot_tui::app::ctrl_j_is_not_swallowed_while_a_turn_runs`

<a id="INPUT-10"></a>
### INPUT-10: a prompt sent while a turn runs waits for it, and says that it is waiting

Enter mid-turn takes the line out of the box and holds it. It is drawn under the box, marked, so
the person can see that what they sent went somewhere. Waiting prompts go in the order they were
typed, one turn each, as soon as the session is free to start one.

A waiting prompt is **not** in the transcript. It has not happened; it moves there at the moment
its own turn begins, and it is drawn as waiting only until then. What it names is settled when it
is queued, not when it is sent, because a file the person took off the line afterwards was never
part of that prompt. It is in the prompt history from the moment it is queued, since from the
person's side that is when they sent it.

Stopping a turn leaves the queue alone. The next waiting prompt begins its turn as it would after
any turn, and the rest go on waiting in order. A prompt is taken back out of the queue by asking
for it (INPUT-18), and until then it goes.

The stopped prompt does **not** come back to the box when something is waiting. It stays in the
transcript as sent, marked stopped, and the box stays empty for whatever the person types next.

**Why.** A stop is aimed at the turn in flight, and nothing else. The prompts behind it are ones
the person typed and has not taken back, so throwing them away made stopping a turn that had gone
wrong cost every prompt they had queued while it went wrong, which is a reason not to press the
key at all.

Un-sending it in front of them would be worse than losing it. The conversation has to read in the
order it happened, and a prompt lifted back out of it while the two typed after it are still
running is in neither place: gone from the transcript, and sitting in a box that is about to be
wanted for the next thing.

Shift-Enter still starts a line rather than sending it, so a paragraph can be written mid-turn and
is not sent half-finished.

**Why.** Enter mid-turn used to reach nothing at all. The line stayed in the box until the person
noticed the turn had ended and pressed it again, which is indistinguishable from a key press that
was ignored. This does not weaken what a running turn refuses: a second turn still cannot begin
while the first is in flight, and the queue is what makes that refusal visible instead of silent.

`verified-by: bravebot_tui::app::enter_queues_a_prompt_while_a_turn_is_running`
`verified-by: bravebot_tui::app::starting_a_line_mid_turn_does_not_queue_it`
`verified-by: bravebot_tui::state::a_prompt_sent_while_a_turn_runs_waits_for_it`
`verified-by: bravebot_tui::state::a_waiting_prompt_goes_when_the_turn_ends`
`verified-by: bravebot_tui::state::waiting_prompts_go_in_the_order_they_were_typed`
`verified-by: bravebot_tui::state::stopping_a_turn_keeps_what_was_waiting_behind_it`
`verified-by: bravebot_tui::state::a_stopped_prompt_stays_sent_where_others_are_waiting`
`verified-by: bravebot_tui::state::a_stopped_prompt_comes_back_where_nothing_is_waiting`
`verified-by: bravebot_tui::state::a_waiting_prompt_is_in_the_history_already`
`verified-by: bravebot_tui::state::there_is_nothing_to_queue_when_the_line_is_blank_or_nothing_is_running`
`verified-by: bravebot_tui::render::a_waiting_prompt_is_shown_as_waiting`
`verified-by: bravebot_tui::render::a_prompt_stops_waiting_once_its_turn_begins`

<a id="INPUT-11"></a>
### INPUT-11: what is attached is drawn nearest the box, above what is waiting

The rows beneath the box run in one order: what the line in the box carries, then the prompts
waiting for the turn in flight, then what the half-typed line could still become.

**Why.** An attachment is part of the line still being composed, and the prompts below it have
already gone. Drawn the other way round, a file staged during a turn sat underneath prompts it
was no part of, which reads as though it went with one of them, and the row for the file the
person had just dropped moved further from the box with every prompt they queued.

`verified-by: bravebot_tui::render::what_is_attached_is_drawn_above_what_is_waiting`
`verified-by: bravebot_tui::render::an_attached_file_is_named_under_the_box`
`verified-by: bravebot_tui::render::a_waiting_prompt_is_shown_as_waiting`

<a id="INPUT-12"></a>
### INPUT-12: an empty box says what it is for

An invitation stands in the empty box, behind the same prompt character a typed line gets and in
the column the first character will land in, with the caret on it. The first thing typed takes its
place and nothing on the row moves. It is drawn rather than typed, so it is never part of a prompt
and never has to be deleted. Shell mode has none: the line there is a command, and its own prompt
character, colour and hint line all say so.

**Why.** An empty box says nothing about what it takes, and the one thing somebody opening this
for the first time needs to know is that they may simply ask.

`verified-by: bravebot_tui::render::an_empty_box_says_what_it_is_for`
`verified-by: bravebot_tui::render::the_invitation_stands_where_the_first_character_will`
`verified-by: bravebot_tui::render::the_invitation_goes_the_moment_anything_is_typed`
`verified-by: bravebot_tui::render::the_invitation_comes_back_when_the_line_does_not`
`verified-by: bravebot_tui::render::the_invitation_is_not_offered_where_the_line_is_a_command`

<a id="INPUT-13"></a>
### INPUT-13: `?` on an empty line lists every key, and the hint line says only that

The marker is a mode rather than a character, as `!` is (INPUT-2, [shell-mode.md](shell-mode.md)):
nothing is typed into the box, the invitation stays where it was, and there is nothing to delete
afterwards. A second `?` takes the list down, as does Escape or typing anything else. Only on an
empty line, since a `?` in a sentence is the punctuation somebody is asking a question with, and in
shell mode it is a glob for the shell to expand.

The list is not a completion. There is nothing in it to choose, so Tab and the arrows go on meaning
what they mean everywhere else while it is up.

It is the one place the keys are written down, so a binding that changes cannot leave the list
advertising something that no longer works. It folds into as many columns as the width holds, and no
row runs past the edge: a row that wrapped would put the list a row over the height reserved for it
and push the hint line off the screen.

The hint line carries what the session is doing (the trail, the confinement, how full the context
is) and then `? for shortcuts`. It lists no binding of its own.

**Why.** The bindings and the state were on one line together, and the line was wider than the
terminal, so the end of it was cut. Everything a person could look up was taking room from the two
figures they had no other way to see. A binding cut off is one somebody learns once; a context
reading cut off is gone. Moving the bindings behind a key they can press when they want them is what
lets the line fit a terminal eighty wide whole.

`verified-by: bravebot_tui::app::a_question_mark_on_an_empty_line_toggles_the_list_without_being_typed`
`verified-by: bravebot_tui::app::a_question_mark_inside_a_sentence_is_punctuation`
`verified-by: bravebot_tui::app::a_question_mark_in_shell_mode_is_a_glob`
`verified-by: bravebot_tui::app::typing_takes_the_list_down`
`verified-by: bravebot_tui::app::escape_takes_the_list_down`
`verified-by: bravebot_tui::render::a_question_mark_lists_every_shortcut`
`verified-by: bravebot_tui::render::the_shortcuts_are_not_something_to_complete`
`verified-by: bravebot_tui::render::the_shortcuts_use_fewer_rows_where_the_width_allows`
`verified-by: bravebot_tui::render::no_shortcut_row_runs_past_the_edge`
`verified-by: bravebot_tui::render::the_hint_line_says_how_to_find_the_bindings_and_reports_confinement`
`verified-by: bravebot_tui::render::the_hint_line_fits_a_narrow_terminal_whole`
`verified-by: bravebot_tui::render::the_hint_and_the_list_name_the_same_key`
`verified-by: bravebot_tui::shell_mode::the_shortcuts_offer_shell_mode`

<a id="INPUT-14"></a>
### INPUT-14: Ctrl-G edits the line in the user's own editor, and only what was saved comes back

The editor opens on what has been typed so far, so the key continues a prompt rather than starting
it again, and what was saved replaces the line. Quitting without saving leaves the line exactly as
it was, and so does an editor that failed or was killed: neither says anything about what the user
wanted. The trailing newline an editor leaves is dropped, one only, and line endings come back the
way a paste's do. The file the editor opened is the user's own words, is readable by nobody else,
and does not outlive the edit down any path. The key does nothing while a turn runs.

**Why.** A prompt worth thinking about outgrows a box ten rows tall with nothing to search or
reflow with. The failure that matters is the one that blanks a paragraph somebody just wrote, so
every path that does not end in a save ends in the line untouched. Handing the terminal to an
editor mid-turn would take the screen from the turn drawing on it.

`verified-by: bravebot_tui::app::ctrl_g_asks_for_the_editor`
`verified-by: bravebot_tui::app::the_editor_key_does_nothing_while_a_turn_runs`
`verified-by: bravebot_tui::state::a_line_from_the_editor_replaces_what_was_typed`
`verified-by: bravebot_tui::editor::the_editor_opens_on_what_was_already_typed`
`verified-by: bravebot_tui::editor::what_the_editor_saved_becomes_the_line`
`verified-by: bravebot_tui::editor::quitting_without_saving_leaves_the_line_as_it_was`
`verified-by: bravebot_tui::editor::an_editor_that_failed_does_not_produce_a_line`
`verified-by: bravebot_tui::editor::the_newline_an_editor_leaves_at_the_end_is_dropped`
`verified-by: bravebot_tui::editor::only_the_last_newline_goes`
`verified-by: bravebot_tui::editor::line_endings_come_back_the_way_a_paste_does`
`verified-by: bravebot_tui::editor::the_file_does_not_outlive_the_edit`

<a id="INPUT-15"></a>
### INPUT-15: `$VISUAL`, then `$EDITOR`, then a list that prefers a full editor to a last resort

An empty value is not an answer, since exporting a variable to nothing is how a profile takes one
back. With neither set, what opens is the first of `vim`, `vi`, `emacs`, `nano` that is installed,
in that order. A configured editor that will not start is reported as such and nothing else is
tried. An editor that returns before the file has been edited is told to wait, but only where the
user wrote no arguments of their own.

**Why.** Someone with `vim` or `emacs` on their machine chose to install it and will not thank a
guess for opening something else; `nano` is the last resort, for the person who has none of them.
Falling back past a name the user exported would run an editor they did not ask for and blame their
configuration for it. A GUI editor that exits the moment its window opens returns the line
unchanged with nothing anywhere saying why, which is neither a failure nor an edit.

`verified-by: bravebot_tui::editor::visual_answers_before_editor`
`verified-by: bravebot_tui::editor::an_empty_variable_is_not_a_configured_editor`
`verified-by: bravebot_tui::editor::a_full_editor_is_preferred_to_the_last_resort`
`verified-by: bravebot_tui::editor::a_configured_editor_that_will_not_start_ends_the_search`
`verified-by: bravebot_tui::editor::a_gui_editor_is_told_to_wait`
`verified-by: bravebot_tui::editor::the_flag_follows_the_program_through_a_path`
`verified-by: bravebot_tui::editor::a_terminal_editor_is_given_no_extra_flag`

<a id="INPUT-16"></a>
### INPUT-16: an editor is started under the name it was asked for

A name is looked for where a shell would look for it, and what runs is that name and not the file a
symlink behind it points at. The name is only kept while it still reaches the same program: a link
that now points elsewhere, or one that no longer resolves, is started by resolved path instead. This
is the editor alone. Everywhere a program is approved before it runs, the approval names the file
that ran, because a name can be repointed afterwards.

**Why.** MacVim installs `vim`, `vi` and `gvim` as links to a single shim that reads its own
`argv[0]`, stays in the terminal for the `vi*` spellings and forks a detached GUI window for the
`m*` and `g*` ones. Started through the resolved path it is always `mvim`, so asking for `vim` opened
a window, returned at once, and put the prompt back unedited with nothing saying why. An editor is
started rather than approved, and for it the name is part of what the user asked for.

`verified-by: bravebot_tui::editor::a_configured_link_to_a_gui_shim_runs_as_the_link`
`verified-by: bravebot_tui::editor::a_terminal_editor_behind_a_gui_shim_is_started_under_its_own_name`
`verified-by: bravebot_tui::editor::a_program_reached_through_a_link_keeps_the_name_it_was_asked_for`
`verified-by: bravebot_tui::editor::the_name_is_looked_for_on_the_path_not_beside_the_resolved_file`
`verified-by: bravebot_tui::editor::an_empty_path_entry_is_not_searched`
`verified-by: bravebot_tui::editor::a_name_that_is_no_longer_the_same_program_falls_back_to_the_resolved_path`

<a id="INPUT-17"></a>
### INPUT-17: Ctrl-S puts the line away, and puts it back

One key, read against the line rather than remembered. A line in the box is put away and the box is
emptied; an empty box is where a line put away earlier comes back, with the caret at its end, where
somebody carries on typing. There is one place to put a line, so a second line put away replaces the
first, and a line that comes back is no longer there to come back again: the next press on the empty
box it left has nothing to do, and says nothing.

**The words travel and the mode does not.** What is put away is what the user typed, and `!` is a
mode rather than a character (INPUT-2, [shell-mode.md](shell-mode.md)), so it stays where they left
it. A prompt comes back into an armed shell as the command they are writing now, and a command comes
back onto an ordinary prompt as words. The list of keys goes, as it does for anything else that
rewrites the box.

**Nothing is sent, so a turn in flight refuses none of it** (INPUT-9). What a line names is settled
when it is sent and not when it is put away, so a marker in a stashed line stands for something still
staged, and names it again when the words holding it come back.

**A line put away says so, on one row beneath the box, and says which key returns it.** One row
however long the line was, above the prompts that are waiting and below what the line in the box
carries (INPUT-11). No row runs past the edge, and where the width will not hold both, the words stay
and the reminder goes: which line is waiting is the part only this row can say, and the key is in the
list `?` puts up as well.

**Why.** A better thought arrives while a worse one is half written, most often during a turn, and
the two ways out were sending the first or losing it. Escape is not a third: it discards, and a
person who wanted the words back has nowhere to have got them from.

The row is the whole of what makes the key safe to press. A press that emptied the box and said
nothing is indistinguishable from one that threw a paragraph away, and the only way to find out
which it had been was to press again and hope. Naming the line is what turns the key from a guess
into a place a thought is being kept.

The caret is not carried. It belongs to an edit that has finished, and restoring it would put a
person back in the middle of a sentence they have not looked at since.

`verified-by: bravebot_tui::app::ctrl_s_puts_the_line_away_and_brings_it_back`
`verified-by: bravebot_tui::app::the_stash_key_works_while_a_turn_runs`
`verified-by: bravebot_tui::state::a_stashed_line_comes_back_as_it_was`
`verified-by: bravebot_tui::state::the_caret_lands_at_the_end_of_a_line_brought_back`
`verified-by: bravebot_tui::state::stashing_again_replaces_what_was_put_away`
`verified-by: bravebot_tui::state::a_line_brought_back_cannot_be_brought_back_again`
`verified-by: bravebot_tui::state::stashing_an_empty_line_with_nothing_put_away_does_nothing`
`verified-by: bravebot_tui::state::the_mode_is_not_stashed_with_the_line`
`verified-by: bravebot_tui::state::a_command_comes_back_as_words_and_not_as_a_command`
`verified-by: bravebot_tui::state::a_line_can_be_stashed_while_a_turn_runs`
`verified-by: bravebot_tui::state::what_a_stashed_line_named_is_still_named_when_it_comes_back`
`verified-by: bravebot_tui::render::a_stashed_line_is_named_under_the_box`
`verified-by: bravebot_tui::render::the_row_goes_when_the_stashed_line_comes_back`
`verified-by: bravebot_tui::render::a_stashed_paragraph_is_one_row`
`verified-by: bravebot_tui::render::what_is_stashed_is_drawn_between_the_attachments_and_the_queue`
`verified-by: bravebot_tui::render::no_stashed_row_runs_past_the_edge`
`verified-by: bravebot_tui::render::a_narrow_terminal_keeps_the_words_and_drops_the_reminder`

<a id="INPUT-18"></a>
### INPUT-18: Up takes back what is waiting before it walks the history

While prompts are waiting, Up puts all of them back into the box in one press, in the order they
were typed, one to a line. Nothing is waiting afterwards, so the rows under the box that said so
go with them. A half-typed line stays below them, where the caret is. What each of them named
comes back staged with it, so a marker in a line that comes back stands for the same file or
picture it stood for when it went. They stay in the prompt history, since from the person's side
they were sent and taking them back does not unsay them.

With nothing waiting the key is unchanged: it walks the history, and scrolls once there is nothing
left to walk. Inside a paragraph it moves between rows first, and reaches the queue from the top
row, the way it reaches the history there.

**Why.** Up is how a person reaches for the last thing they said, and while something is waiting
the last thing they said is in the queue. The history holds a copy of every queued line from the
moment it is queued (INPUT-10), so the key handed back a copy: the person rewrote it, sent it, and
the original went as well. The only way to take a queued prompt back was to stop the turn in
flight, which is aimed at something else entirely and costs the answer being written.

`verified-by: bravebot_tui::app::up_takes_back_everything_waiting_rather_than_a_copy_of_it`
`verified-by: bravebot_tui::app::up_walks_the_history_again_once_nothing_is_waiting`
`verified-by: bravebot_tui::state::taking_the_queue_back_puts_every_waiting_prompt_in_the_box`
`verified-by: bravebot_tui::state::a_half_typed_line_stays_below_what_comes_back`
`verified-by: bravebot_tui::state::what_a_waiting_prompt_named_is_named_again_when_it_comes_back`
`verified-by: bravebot_tui::state::there_is_nothing_to_take_back_when_nothing_is_waiting`
`verified-by: bravebot_tui::render::the_waiting_rows_go_when_the_queue_is_taken_back`

## Known costs

- **A stopped request leaves a thread and a socket behind.** The reply goes on being read by
  nobody until the far end finishes or the connection times out, which can be minutes on a
  connection that has died. It costs a thread and a socket for that long, and it is the price of
  answering the person at once: the alternative is waiting for a read that cannot be interrupted.
  Nothing that thread does is visible, since it holds no policy, no workspace and no tool.
- **Asking the terminal about itself at startup can eat what was typed into that moment.** The
  question about the background is asked once, before the first frame, and the answer is read off
  the tty directly. Anything typed or pasted in the window before the answer arrives is read by
  that same call and discarded, and a terminal that answers with nothing keeps the window open for
  its full 80 ms. It is bounded, it is once per session, and it is before there is a box to type
  into, which is why it is a cost and not a clause.
- **Ctrl-S is the byte a terminal traditionally freezes its output with.** It reaches this process
  because raw mode turns that flow control off for as long as the session holds the terminal, which
  is what makes the chord bindable at all (INPUT-17). The cost is a person's muscle memory: somewhere
  behind a `tmux` or `screen` configured to keep flow control, or an ssh session that does, the key
  can be taken before it arrives, and then it does nothing here. Nothing is lost when that happens,
  since the line stays in the box.
