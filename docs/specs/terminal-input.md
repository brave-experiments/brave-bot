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

**Why.** It used to say "cancelling…" and go on streaming the reply to the end, because a stop was
only noticed between rounds. So the key that was supposed to stop the answer left the answer
running and put a progress report on the screen about a key press, and the longer the reply the
longer somebody waited for the thing they had already stopped. A tool call already running still
finishes, since stopping one part way could leave a file half written, but nothing new starts.

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

Ctrl-G opens `$VISUAL` or `$EDITOR` and takes back what was saved, and does nothing while a turn
runs.

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
`verified-by: bravebot_tui::state::cancelling_before_anything_happens_still_un_sends_the_prompt`
`verified-by: bravebot_tui::app::a_key_that_would_stop_a_turn_is_answered_during_a_summary`
`verified-by: bravebot_tui::app::escape_stops_the_turn_without_ending_the_session`
`verified-by: bravebot_tui::app::ctrl_g_asks_for_the_editor`
`verified-by: bravebot_tui::app::the_editor_key_does_nothing_while_a_turn_runs`


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

A difference between the two has to be a difference about sending. Anything else is drift, and
this one was silent, so the two paths walk one list of keys rather than a list each.

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
any turn, and the rest go on waiting in order.

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

An invitation stands in the empty box, with the caret on its first character, and the first thing
typed takes its place. It is drawn rather than typed, so it is never part of a prompt and never
has to be deleted. Shell mode has none: the line there is a command, and its own prompt character,
colour and hint line all say so.

**Why.** An empty box says nothing about what it takes, and the one thing somebody opening this
for the first time needs to know is that they may simply ask.

`verified-by: bravebot_tui::render::an_empty_box_says_what_it_is_for`
`verified-by: bravebot_tui::render::the_invitation_goes_the_moment_anything_is_typed`
`verified-by: bravebot_tui::render::the_invitation_comes_back_when_the_line_does_not`
`verified-by: bravebot_tui::render::the_invitation_is_not_offered_where_the_line_is_a_command`
