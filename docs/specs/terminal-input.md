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

`verified-by: bravebot_tui::drop::deleting_the_marker_takes_the_attachment_off`
`verified-by: bravebot_tui::drop::several_files_dropped_together_each_get_a_marker`
`verified-by: bravebot_tui::drop::sending_a_line_clears_what_was_attached_to_it`
`verified-by: bravebot_tui::render::an_attached_file_is_named_under_the_box`


<a id="INPUT-4"></a>
### INPUT-4: escape clears a line before it quits

Escape on a typed line clears it without quitting; on an empty line it quits. Escape and Ctrl-C
both cancel a running turn. Ctrl-G opens `$VISUAL` or `$EDITOR` and takes back what was saved, and
does nothing while a turn runs.

`verified-by: bravebot_tui::app::escape_clears_a_typed_line_without_quitting`
`verified-by: bravebot_tui::app::escape_quits_on_an_empty_line`
`verified-by: bravebot_tui::app::ctrl_c_quits`
`verified-by: bravebot_tui::app::cancel_keys_are_escape_and_ctrl_c`
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
