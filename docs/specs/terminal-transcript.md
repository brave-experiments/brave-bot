---
id: VIEW
title: The transcript
status: normative
governs:
  - crates/tui/src/render.rs
  - crates/tui/src/state.rs
---

## Scope

What is drawn back to the user: the transcript, a resumed session, and how content is shaped on
its way to the screen. Presentation holds no labels, and the rules here are about a person being
able to see what the agent did. What the user types into is
[terminal-input.md](terminal-input.md).

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
