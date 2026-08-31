---
id: VIEW
title: The transcript
status: normative
governs:
  - crates/tui/src/render.rs
  - crates/tui/src/state.rs
  - crates/tui/src/theme.rs
  - crates/tui/src/theme_prompt.rs
---

## Scope

What is drawn back to the user: the transcript, a resumed session, how content is shaped on its
way to the screen, and which palette paints the interface. Presentation holds no labels, and the
rules here are about a person being able to see what the agent did. What the user types into is
[terminal-input.md](terminal-input.md). A line beginning with `/` is [commands.md](commands.md).

## Clauses

<a id="VIEW-1"></a>
### VIEW-1: the end of a reply is visible when it arrives, and scrolling back is deliberate

A wrapped reply shows its end as it lands. Scrolling back changes the view and holds it there.

`verified-by: bravebot_tui::render::the_end_of_a_wrapped_reply_is_visible_when_it_arrives`
`verified-by: bravebot_tui::render::scrolling_back_changes_the_view`


<a id="VIEW-2"></a>
### VIEW-2: a resumed session shows what the earlier turns did

The trail, the plan worked to, the calls made, and what has been spent all come back, so reading a
transcript back does not depend on remembering the session.

`verified-by: bravebot_tui::state::a_resumed_turn_shows_the_trail_it_left`
`verified-by: bravebot_tui::state::a_resumed_turn_shows_the_plan_it_worked_to`
`verified-by: bravebot_tui::state::a_resumed_transcript_shows_the_calls_the_turn_made`
`verified-by: bravebot_tui::state::a_resumed_session_carries_on_counting_what_it_has_spent`
`verified-by: bravebot_agent::conversation::a_recounted_turn_says_what_it_did_and_not_only_what_it_said`
`verified-by: bravebot_agent::conversation::every_call_in_a_round_is_recounted`
`verified-by: bravebot_agent::conversation::what_a_call_returned_is_not_recounted`
`verified-by: bravebot_agent::conversation::a_call_with_unreadable_arguments_is_still_recounted`


<a id="VIEW-3"></a>
### VIEW-3: untrusted content is shown on purpose, inside a margin it cannot forge

Showing it is deliberate and not a leak. Filenames out of a quarantined listing, the first lines of
a file nobody vouched for, what a processor produced, the body of every write: all of it reaches
the person watching, because an agent that will not say which file it is working on has protected
nobody. It is the planner that may not read untrusted content, and a terminal is not a planner's
context. It reaches a screen under a witness minted for release to a display and for nothing else.

What must hold is how it is drawn. A bar runs down the margin of every **drawn row** of the block,
and the content never gets to draw its own, so a file containing "untrusted content ends here" ends
nothing. A caption can be imitated by the thing it captions; a margin cannot. Never replace the bar
with a heading, and never show untrusted content outside a marked block.

Rows rather than lines, because a line longer than the terminal is wide becomes several of them. It
is broken to the width by the same step that draws the margin, and each row it breaks into carries
a bar of its own. Leaving the break to the paragraph the block is drawn in is the same defect as
omitting the margin: the continuation starts at column 0, which is untrusted content outside the
block, positioned wherever the content's own padding chose to put it. Nothing is dropped to make a
line fit. The block's heading is laid out the same way and for the same reason, since the origin
named in it can be a filename read out of a quarantined listing.

Every control character is replaced with a visible glyph on the way to the screen, in the heading
as well as the content. Replaced rather than dropped, since a character silently removed is one the
user cannot tell was ever in the file.

`verified-by: bravebot_tui::marking::quarantined_content_cannot_paint_its_own_margin`
`verified-by: bravebot_tui::marking::a_neutralised_escape_is_still_visible`
`verified-by: bravebot_tui::marking::text_without_control_characters_is_drawn_as_it_is`
`verified-by: bravebot_tui::render::quarantined_content_is_shown_and_marked_on_every_line`
`verified-by: bravebot_tui::marking::a_wrapped_preview_line_is_marked_on_every_row_it_reaches`
`verified-by: bravebot_tui::marking::wrapped_content_cannot_paint_a_bar_in_the_margin_column`
`verified-by: bravebot_tui::marking::a_long_origin_keeps_the_heading_inside_the_block`
`verified-by: bravebot_agent::turn::quarantined_content_reaches_the_person_and_not_the_planner`

<a id="VIEW-4"></a>
### VIEW-4: untrusted content is never drawn as structure

A quarantined preview is never drawn as a table. Everything untrusted goes through the margin and
control-character replacement of VIEW-3. Shell mode output is **not** drawn as quarantined,
because it is trusted, and it still cannot draw its own escapes.

`verified-by: bravebot_tui::render::a_quarantined_preview_is_never_drawn_as_a_table`
`verified-by: bravebot_tui::shell_mode::output_is_not_drawn_as_quarantined`
`verified-by: bravebot_tui::shell_mode::output_cannot_draw_its_own_escapes`


<a id="VIEW-5"></a>
### VIEW-5: a tiny terminal still renders

Every prompt and the session view render at small sizes rather than panicking or truncating the
question out of view.

`verified-by: bravebot_tui::trust_prompt::a_tiny_terminal_still_renders`
`verified-by: bravebot_tui::confirm::a_tiny_terminal_still_renders_the_prompt`

<a id="VIEW-6"></a>
### VIEW-6: a reply is drawn as it arrives, and the round that ends replaces it

The words are drawn where the finished entry will be and in the shape it will have, so nothing on
the screen moves when the round ends. A round that finishes with nothing to say takes its own tail
down, and so does a turn that fails, is stopped, or has to send its request again: what an
abandoned attempt had written is no part of the reply that replaces it.

**Why.** The longest silence in a turn is the one while the model writes, and it is the silence
with the most to show. A counter reports that something is happening; it does not report what.

The words are untrusted model output released for a screen, on the same footing as everything else
in this file and through the same gate. Released once for the round rather than once per frame,
because it is one release however many pieces it arrives in.

`verified-by: bravebot_tui::render::a_reply_is_drawn_while_it_is_still_arriving`
`verified-by: bravebot_tui::render::a_reply_looks_the_same_arriving_as_it_does_arrived`
`verified-by: bravebot_tui::state::a_streamed_reply_grows_rather_than_being_replaced`
`verified-by: bravebot_tui::state::the_finished_round_takes_over_from_the_reply_that_was_arriving`
`verified-by: bravebot_tui::state::a_reply_that_was_arriving_is_taken_down_however_the_round_ends`
`verified-by: bravebot_tui::state::a_round_starting_afresh_starts_from_an_empty_tail`
`verified-by: bravebot_agent::turn::the_reply_reaches_the_interface_while_it_is_being_written`
`verified-by: bravebot_agent::turn::showing_a_reply_as_it_arrives_is_recorded_once_for_the_round`

<a id="VIEW-7"></a>
### VIEW-7: where a result went is drawn only where that is not the ordinary answer

A call whose result the planner may read says nothing about it. A result the planner may not read
says so, and so does a name that was never opened.

**Why.** Nearly every call reads into the planner's context, and a line under nearly every call
distinguishes nothing while crowding out the lines that do. What the design turns on is the
exception, and the exception is still marked twice over: on the call, and again in the margin of
the block its content is drawn in.

Recording is unaffected. What is dropped here is a row on a screen, not a fact: where every result
went is still in the audit trail, which is what the record is for.

`verified-by: bravebot_tui::render::the_ordinary_landing_is_not_given_a_line_of_its_own`
`verified-by: bravebot_tui::render::a_result_the_planner_may_not_read_still_says_so`

<a id="VIEW-8"></a>
### VIEW-8: a note from the session is drawn in one ink of its own

What the session says in its own voice, the trust answer, an unavailable confinement, a status
report, is drawn in a single ink belonging to nothing else, and never in the ink that marks
untrusted content.

**Why.** That ink is spoken for twice over: a call still running, and the margin down every block
of content the planner may not read. Drawing a note in it said the trust answer was quarantined.
An ink of its own rather than merely a different one, because a note sharing with any third meaning
puts the question back where it started.

This is about which ink, and it is never what makes the marking hold. VIEW-3 stands on the margin
because a colour can be imitated by the content beside it, and nothing here weakens that: no ink
tells a reader whether something is quarantined, and a note drawn in the wrong one would still be
outside a block.

`verified-by: bravebot_tui::render::a_system_note_is_not_drawn_in_the_ink_that_marks_untrusted_content`

<a id="VIEW-9"></a>
### VIEW-9: under `brave`, an ink that carries meaning is mixed, not chosen by the terminal

Where a colour is what tells one thing on the screen from another, and the theme in force is
`brave`, it is a shade this interface mixes. The sixteen named colours are slots a terminal
repaints, so they are used only where the meaning is the terminal's own, green for finished, red
for failed, dim grey for an aside, which are read against whatever palette the user chose rather
than against each other. A mixed shade that has to stay legible against the background is picked
for the background sensed at startup, and a terminal that will not say gets the shade for a dark
one.

**Why.** A named slot is a request, not a colour. The same code drew a different colour in every
profile, which is how one slot came to carry two meanings at once without anybody choosing that.

`verified-by: bravebot_tui::theme::a_note_is_a_shade_and_not_a_slot_a_terminal_repaints`
`verified-by: bravebot_tui::theme::brand_primary_is_a_shade_and_not_a_slot_a_terminal_repaints`
`verified-by: bravebot_tui::theme::a_dark_background_takes_the_brighter_brand_primary`
`verified-by: bravebot_tui::theme::a_light_background_takes_the_deeper_brand_primary`
`verified-by: bravebot_tui::theme::colorfgbg_with_a_white_background_is_light`
`verified-by: bravebot_tui::theme::an_osc_reply_with_a_pale_background_is_light`
`verified-by: bravebot_tui::theme::brave_keeps_named_slots_for_the_terminals_own_meanings`

<a id="VIEW-10"></a>
### VIEW-10: a palette a person chose paints every role from that table

A theme chosen with `/theme` mixes every semantic ink from the palette that name names, including
the background and the default text. Named ANSI slots are not used there, so two roles cannot
collapse because the terminal remapped green. The choice is a keystroke on this surface, the same
endorsement `/model` takes for a request field. Moving the cursor live-previews: the theme under
the cursor is put in force for as long as it is selected, and Escape restores the theme that was
in force when the picker opened.

**Why.** Leaving finished and failed as named slots under a named theme would put the person's
chosen palette and the terminal's remapping in a fight over the same meaning. Previewing on the
cursor rather than only on Enter is what lets a person compare themes against their own transcript
before committing.

`verified-by: bravebot_tui::theme::a_named_theme_paints_its_own_background_and_inks`
`verified-by: bravebot_tui::theme_prompt::preview_puts_the_cursor_theme_in_force_and_cancel_restores`
`verified-by: bravebot_tui::app::typing_the_theme_command_opens_the_picker`
`verified-by: bravebot_tui::app::the_theme_command_carries_its_name`

<a id="VIEW-11"></a>
### VIEW-11: user theme files are read only from `~/.bravebot/themes`

A JSON file whose stem is the theme name is loaded from the user's own themes directory and from
nowhere else. A workspace `.bravebot/themes` is not consulted: that directory is workspace content,
and a palette file must not become a decision taken from untrusted bytes. An unknown or unreadable
stored name is `brave`. A broken file is omitted from the list rather than crashing. The earlier
name `system` still finds `brave`, so a choice saved under that name is not silently lost.

`verified-by: bravebot_tui::theme::user_themes_come_from_a_directory_of_json_files`
`verified-by: bravebot_tui::theme::a_broken_json_file_is_not_a_theme`
`verified-by: bravebot_tui::theme::none_in_json_inherits_the_terminal_default`
`verified-by: bravebot_tui::theme::the_old_system_name_still_finds_brave`
`verified-by: bravebot_tui::store::an_empty_theme_file_is_not_a_choice`
`verified-by: bravebot_tui::store::an_over_long_theme_name_is_not_a_choice`

<a id="VIEW-12"></a>
### VIEW-12: the theme picker is a centred panel over the session

`/theme` draws a bordered panel in the middle of the screen. The session stays visible behind it,
and is redrawn each time the cursor moves so the live preview is of the person's own transcript
rather than of an empty page. The panel is sized to the list and stays inside the frame on a tiny
terminal. It is not a full-screen takeover.

**Why.** A full-screen list hides the thing a theme is for. The same centred-panel shape the write
and trust prompts already use keeps the person oriented, and putting the session behind the panel
is what makes previewing honest.

`verified-by: bravebot_tui::theme_prompt::the_picker_is_drawn_as_a_centred_panel`
`verified-by: bravebot_tui::theme_prompt::the_panel_stays_inside_a_tiny_terminal`
`verified-by: bravebot_tui::theme_prompt::the_list_shows_names_a_person_reads`
