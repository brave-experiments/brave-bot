---
id: SCROLL
title: The scroller
status: normative
governs:
  - crates/tui/src/app.rs
  - crates/tui/src/render.rs
  - crates/tui/src/state.rs
  - crates/tui/src/editor.rs
---

## Scope

Reading back through what has already happened: the mode Ctrl-O opens over the transcript, the
keys it answers, and the two ways out of the session it offers. What is drawn, and how untrusted
content is marked wherever it appears, is [terminal-transcript.md](terminal-transcript.md). What
the user types into at rest is [terminal-input.md](terminal-input.md).

The transcript already scrolls: the wheel, the arrows and the page keys move it, and a scrolled
view is held rather than dragged back to the tail. What is missing is everything a person does
once they are reading rather than typing, which is to jump, to search, to see the part that was
trimmed, and to get text out. Those need keys, and the keys they need are letters, so they need a
mode.

## Clauses

<a id="SCROLL-1"></a>
### SCROLL-1: Ctrl-O opens the scroller on the view already on the screen, and four keys close it

Opening moves nothing. The scroller shows the same transcript at the same offset, so the row the
person was looking at when they pressed the key is the row under their eyes afterwards. It is one
view with two sets of keys over it, not a second copy of the transcript. The screen does change
shape underneath it, since the box comes off and the transcript is given its rows, and the row at
the top of the view is held across that: what is gained appears below what is already drawn, and
what is given back when the scroller closes is covered by the box coming home.

`q`, Escape and Ctrl-O each close it, and the view stays where the scroller left it. Escape
clears a standing search first, since that is nearer.

Ctrl-C closes it as well, and does nothing else. The scroller is the nearest thing there is to
stop, so a turn in flight goes on running and the press that reaches it is the next one. Each
press has something of its own to answer and the screen says which, so no press silently does
another one's job.

**Why.** A viewer that jumps to the bottom on the way in has lost the thing the person opened it
to look at. Escape stops the nearest thing and never leaves; Ctrl-C stops the nearest thing and
leaves once there is nothing left. An open scroller is a thing to stop, and it is the innermost
one.

`verified-by: bravebot_tui::app::ctrl_o_opens_the_scroller`
`verified-by: bravebot_tui::app::the_scroller_opens_on_the_view_that_was_already_there`
`verified-by: bravebot_tui::app::q_escape_and_ctrl_o_each_close_the_scroller`
`verified-by: bravebot_tui::app::closing_the_scroller_leaves_the_view_where_it_was`
`verified-by: bravebot_tui::state::the_row_at_the_top_of_the_view_survives_the_screen_changing_shape`
`verified-by: bravebot_tui::app::ctrl_c_closes_the_scroller_before_it_reaches_anything_else`
`verified-by: bravebot_tui::app::the_scroller_answers_the_stop_keys_before_the_turn_does`
`verified-by: bravebot_tui::app::a_turn_goes_on_running_while_the_scroller_is_open`


<a id="SCROLL-2"></a>
### SCROLL-2: while it is open the keys are the scroller's, and the line in the box is untouched

Every key in this file is answered here, and a key this file does not name does nothing at all. A
character does not reach the box, Enter sends nothing, and nothing typed while the scroller is open
becomes part of a prompt.

The line keeps its text, its caret and whatever is attached to it, and comes back exactly as it was
the moment the scroller closes.

**Why.** The box takes letters, and so does a pager. One of them has to give. A mode that leaks
its keystrokes into a box the person cannot see is the worse half of both: `j` scrolls and also
types a `j`, and the prompt they had half written is quietly a different prompt by the time they
come back to it.

Nothing here is a difference about sending. Sending is what a running turn refuses; this is a mode
the person opened with a key and closes with any of four.

`verified-by: bravebot_tui::app::a_typed_character_does_not_reach_the_box_while_the_scroller_is_open`
`verified-by: bravebot_tui::app::enter_sends_nothing_from_inside_the_scroller`
`verified-by: bravebot_tui::app::a_key_the_scroller_does_not_take_does_nothing`
`verified-by: bravebot_tui::app::the_line_comes_back_untouched_when_the_scroller_closes`


<a id="SCROLL-3"></a>
### SCROLL-3: the keys that move the view

| Keys | Where the view goes |
|---|---|
| Up / Down, `k` / `j` | one line back / on |
| Ctrl-U / Ctrl-D | half a screen back / on |
| Space / `b`, Ctrl-F / Ctrl-B | a whole screen on / back |
| `g` / `G`, Home / End | the first row / the last |
| `{` / `}` | the prompt before this one / the prompt after |
| the wheel | what it does at rest, stopping at the ends as everything here does |

Each end is a stop rather than a count that keeps going: neither direction moves past the first row
or the last, so a held key comes to rest somewhere the next press can move away from.

`{` and `}` land on the row a turn begins at, which is a prompt the person typed. Where they land
is settled by what the person wrote and by nothing read out of the workspace.

**Why.** Two dialects, because the people who reach for a pager have `less` or `vi` in their hands
already and neither group should have to learn the other's. A key that does nothing on arrival
reads as a broken feature, and the cost of answering both is a row in this table.

`verified-by: bravebot_tui::app::the_line_keys_move_the_view_by_a_line`
`verified-by: bravebot_tui::app::the_half_page_keys_move_the_view_by_half_a_screen`
`verified-by: bravebot_tui::app::the_page_keys_move_the_view_by_a_whole_screen`
`verified-by: bravebot_tui::app::g_and_shift_g_reach_the_first_row_and_the_last`
`verified-by: bravebot_tui::app::the_prompt_keys_land_on_the_turn_before_and_the_turn_after`
`verified-by: bravebot_tui::app::the_view_stops_at_the_first_row_rather_than_scrolling_past_it`
`verified-by: bravebot_tui::app::the_view_stops_at_the_last_row_rather_than_scrolling_past_it`
`verified-by: bravebot_tui::app::the_wheel_scrolls_the_scroller_as_it_scrolls_the_transcript`


<a id="SCROLL-4"></a>
### SCROLL-4: `/` searches what is drawn, literally, and `n` and `N` walk the matches

The needle is typed at the foot of the screen. Enter runs it, and Escape abandons it and leaves
the view where it was. Escape against a search that has already run clears it instead: the
highlights come off and the view stays, and the press after that is the one that closes the
scroller. It is the ladder every stop key in this interface walks, which is that a press answers
the nearest thing there is to stop.

It is matched as a substring, character for character, and never as a pattern: case-insensitive
while the needle is all lower case, exact from the moment it holds a capital. What is searched is
the text of the rows as they are drawn, so a match is always something the person can see: where a
block replaces control characters with visible glyphs on its way to the screen, the glyphs are
what a needle meets and the bytes behind them are not there to be found.

Every match is drawn highlighted where it already is. `n` and `N` move to the next and the previous
and wrap at the ends, and how many matches there are is drawn. A needle that matches nothing says
so and moves nothing.

**Why.** A pattern language is an interpreter reached by a line the person types over text an
attacker may have written, and a backtracking one is a stall waiting to be found. Literal matching
is also what somebody scanning a transcript for a filename actually wants.

`verified-by: bravebot_tui::state::a_search_matches_a_substring_literally`
`verified-by: bravebot_tui::state::a_needle_in_lower_case_matches_either_case`
`verified-by: bravebot_tui::state::a_needle_holding_a_capital_matches_exactly`
`verified-by: bravebot_tui::state::a_pattern_is_matched_as_the_characters_it_is_spelled_with`
`verified-by: bravebot_tui::render::a_search_matches_what_is_drawn_and_not_the_bytes_behind_it`
`verified-by: bravebot_tui::state::n_and_shift_n_walk_the_matches_and_wrap`
`verified-by: bravebot_tui::render::a_search_that_matches_nothing_says_so_and_moves_nothing`
`verified-by: bravebot_tui::app::escape_abandons_a_half_typed_search`
`verified-by: bravebot_tui::app::escape_clears_a_finished_search_before_it_closes_the_scroller`
`verified-by: bravebot_tui::app::backspacing_past_the_start_abandons_the_search`
`verified-by: bravebot_tui::render::every_match_on_the_screen_is_highlighted`
`verified-by: bravebot_tui::render::how_many_matches_there_are_is_drawn`


<a id="SCROLL-5"></a>
### SCROLL-5: a search matches untrusted content too, and never lifts it out of its block

Quarantined content is searched along with everything else on the screen, and a match inside a
quarantined block is highlighted inside that block. The footer says how many matches there are and
which one the view is on. It never quotes one.

Every row of a quarantined block carries the margin in the scroller exactly as it does in the
transcript, under full detail as much as under a preview, and content still cannot draw a bar of
its own.

**Why the footer says a number and not a line.** A quoted match is untrusted content drawn outside
a marked block, in the one row of the screen the interface speaks in its own voice. A count cannot
be forged into a sentence; a quotation is a sentence already.

**Why search it at all.** An interface that shows content and then refuses to let a person find
it has protected nobody and made the audit worse. This is a decision taken from untrusted bytes,
which the rest of the system does not do, so what it can and cannot buy an attacker is written out
under Known costs rather than left to be found.

`verified-by: bravebot_tui::render::a_search_matches_quarantined_content_too`
`verified-by: bravebot_tui::render::a_match_inside_a_quarantined_block_stays_inside_it`
`verified-by: bravebot_tui::render::the_search_footer_never_quotes_what_it_matched`
`verified-by: bravebot_tui::render::a_quarantined_row_is_marked_in_the_scroller_as_it_is_in_the_transcript`


<a id="SCROLL-6"></a>
### SCROLL-6: `v` opens the transcript in the user's editor, and reads nothing back

What goes is the rows as they are drawn, margins and all, so untrusted content is marked in the
file the way it is marked on the screen. It goes to a temporary file outside the workspace, opened
with `$VISUAL` or `$EDITOR` the same way a prompt is, and the file goes when the editor exits. It
does nothing while a turn runs, which is the answer the key that edits a prompt already gives: an
editor needs the screen, and a running turn is drawing it.

Nothing comes back. The key that edits a prompt takes back what was saved, because a prompt is a
thing the person is still writing; a transcript is a record of what happened, and a record that can
be edited into the session is not one. No later turn reads the file, and no path in the workspace
gains anything from it having existed.

**Why.** A pager can search a screen. Everything past that, meaning reading two passages side by
side, keeping a copy, or grepping the lot, is a text editor's job, and the user has one.

`verified-by: bravebot_tui::app::v_asks_for_the_editor`
`verified-by: bravebot_tui::app::the_transcript_editor_key_does_nothing_while_a_turn_runs`
`verified-by: bravebot_tui::render::what_goes_to_the_editor_is_marked_the_way_the_screen_is`
`verified-by: bravebot_tui::editor::a_transcript_opened_in_the_editor_is_written_outside_the_workspace`
`verified-by: bravebot_tui::editor::a_transcript_opened_in_the_editor_is_never_read_back`
`verified-by: bravebot_tui::editor::the_file_goes_when_the_editor_exits`


<a id="SCROLL-7"></a>
### SCROLL-7: the scroller says it is open, and `?` says what it takes

A footer stands while the scroller is open, saying so and naming a key that closes it. `?` lists
every key in this file, and the list renders on a terminal too short for it rather than pushing the
way out off the screen: what a short terminal loses is rows from the middle, never the last one.

The list is read instead of the transcript rather than alongside it, so any key at all puts it
away and that press is spent doing so. The list says as much, because a key that quietly did two
things would be worse than one that does the obvious one.

**Why.** A mode where the letters a person types do nothing, with nothing on the screen to say why,
is indistinguishable from an interface that has stopped responding. The way out is the one line
that must never be the line that did not fit.

`verified-by: bravebot_tui::render::the_scroller_says_it_is_open_and_which_key_closes_it`
`verified-by: bravebot_tui::app::the_help_key_lists_the_keys`
`verified-by: bravebot_tui::render::the_help_renders_on_a_tiny_terminal`


<a id="SCROLL-8"></a>
### SCROLL-8: a turn goes on underneath, and the view does not move to follow it

What arrives while the scroller is open joins the transcript and does not drag the view to the
tail, in the frame it arrives in and not one frame later. The footer says that more has arrived
below and how much, and `G` reaches it. It says that a turn is still running as well, in the word
the indicator would have used, since the indicator is not on the screen for it to say so itself.

**Why.** Holding the view is the whole of what the scroller is for. A person reading back through a
turn that is going wrong is reading precisely because it is going wrong, and a view yanked to the
bottom by the next line the model writes takes away the only thing they were trying to do.

`verified-by: bravebot_tui::state::what_arrives_while_the_scroller_is_open_does_not_move_the_view`
`verified-by: bravebot_tui::render::a_turn_writing_underneath_does_not_slide_the_view_between_frames`
`verified-by: bravebot_tui::render::the_scroller_says_more_has_arrived_below`
`verified-by: bravebot_tui::render::the_scroller_says_a_turn_is_still_running`
`verified-by: bravebot_tui::state::the_last_row_reached_from_the_scroller_includes_what_arrived`

<a id="SCROLL-9"></a>
### SCROLL-9: the scroller has the whole screen but one row

While it is open the transcript is drawn on every row of the screen except the last, which is the
footer. The box goes, the indicator above it goes, and anything being offered beneath it goes. All
of them come back the moment it closes.

**Why.** Every one of those is a thing a person is invited to type at, and no key reaches any of
them from in here: a box drawn under a mode that cannot reach it is rows of the screen spent
inviting a keystroke that would do nothing, and a caret blinking in it is the interface saying the
opposite of what is true. What they cost is given to the transcript, which is the whole of what
somebody opened a pager to look at.

`verified-by: bravebot_tui::render::the_scroller_takes_the_whole_screen_but_its_own_footer`
`verified-by: bravebot_tui::render::the_usual_hint_comes_back_when_the_scroller_closes`

## Known costs

- **A search is a decision taken from untrusted bytes.** Nothing else in the driver does this, so
  it is written down here rather than left to be discovered by somebody reading the code. Where the
  view lands after `/` is derived from text that may have come out of a file nobody vouched for.

  What that buys an attacker is the whole of this list:

  - **Add matches.** Content can hold whatever a person is likely to search for, so a search for
    `.env` can be answered by forty planted hits with the real one somewhere among them. Somebody
    who gives up before walking them all has been made to give up. That is the real cost here, and
    it is bounded by the count: `n` reaches every match, the footer says how many there are, and a
    transcript with forty of something is itself a thing worth looking at.
  - **Be found first.** Which match the view lands on first is the one nearest the view, so content
    can arrange to be the first thing a person sees. It cannot be the only thing.

  What is not on the list is the part that would matter. Content cannot remove a match, cannot stop
  `n` reaching one, cannot quote itself into the footer, and cannot leave a quarantined block: a
  match inside one is highlighted inside it, behind the same margin every other row of it carries.
  Nothing is routed by this, nothing is written, nothing leaves the process, and no model is given
  a byte of it. The view moves and the view is the only thing that moves.
