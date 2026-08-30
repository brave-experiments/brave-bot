---
id: PASTE
title: Pasting
status: normative
governs:
  - crates/tui/src/clipboard.rs
  - crates/tui/src/state.rs
  - crates/tui/src/app.rs
guards:
  - symbol: Policy::admit_pasted_image
---

## Scope

What Ctrl-V puts into a turn, whether it is text or a picture, and on what footing. Pasted text is
presentation and holds no labels: it is the user's own words either way, and the only question is
that the box stays readable. A pasted picture is a claim about provenance, and most of this spec is
about that claim. A paste leaves a marker in the box, and deleting the marker takes the paste off.

Three gestures put content into a turn on the user's own footing, and each has its own spec:
[naming-files.md](naming-files.md) for `@` in a prompt, [pasting.md](pasting.md) for Ctrl-V, and
[dropping.md](dropping.md) for a file dragged onto the window.

## Why a pasted picture is trusted

A pasted picture is trusted because the user pasted it, exactly as they typed the words beside it.
It arrives in their own message, on the footing of the prompt it came with, and the gate that
admits it records that provenance and asserts nothing else. Nothing inspects the pixels and nothing
could. Being a picture establishes nothing: the trust comes from the gesture and from nowhere else.

It is carried whole rather than quarantined because a processor takes text slots and returns text,
so there is no reference an image could be reduced to. That is a fact about the mechanism, and not
a second reason to trust it.

## Clauses

### PASTE-1: a long paste folds to a marker, and the words are what get sent

More than a couple of lines folds to `[Pasted text #2 +40 lines]`, counting the lines a person
would count. The words around it are left alone, the text is put back before the turn is built, and
a short paste lands whole. A paste into a command line is never folded. A paste ending in a newline
does not send.

**Why.** A stack trace would otherwise push the reply being read off the screen. Nothing is hidden:
what is about to be sent is what the prompt says, and deleting the marker drops the words.

`verified-by: bravebot_tui::state::a_folded_paste_counts_the_lines_a_person_would_count`
`verified-by: bravebot_tui::state::a_folded_paste_leaves_the_words_around_it_alone`
`verified-by: bravebot_tui::state::a_folded_paste_is_put_back_before_the_turn_is_built`
`verified-by: bravebot_tui::state::a_short_paste_lands_in_the_box_whole`
`verified-by: bravebot_tui::state::a_paste_into_a_command_line_is_never_folded`
`verified-by: bravebot_tui::render::a_folded_paste_keeps_its_lines_off_the_screen`
`verified-by: bravebot_tui::app::a_paste_that_ends_in_a_newline_does_not_send_it`
`verified-by: bravebot_tui::app::a_pasted_prompt_is_sent_when_the_user_says_so`


### PASTE-2: only a picture a human pasted

Never bytes a tool read, never anything a processor produced, never an image a path in model
output named. Each of those is content, and routing it here would launder it.

**Why.** The justification cannot be checked from the bytes, so it lives at the call site. Today
that is the TUI's Ctrl-V and nothing else.

`verified-by: bravebot_tui::app::a_picture_off_the_clipboard_becomes_a_marker_in_the_line`


### PASTE-3: the media type is the driver's, never the content's

It ends up in the data URL, where it is routing, so it comes from a fixed set
the clipboard reader owns, and never from a filename or from what a tool printed.

`verified-by: none`


### PASTE-4: the picture is inlined, never linked

A URL would have the endpoint fetch it over a connection this process never makes, which is an
egress `bravebot-net` could not gate.

`verified-by: none`


### PASTE-5: a paste does not lower context integrity

It says nothing about content the planner has met. Lowering it here would have a screenshot mark
everything the planner then wrote as untrusted, on the strength of the user's own input.

`verified-by: none`


### PASTE-6: what is sent is what the prompt says

The marker is written where the caret is and the picture goes wherever that text goes. Deleting
the marker unsends it; recalling an older prompt carries none of them, because the markers went
with the line. A picture is refused in shell mode rather than written into the command. Anything
over 10 MB is refused rather than sent, and says so with its size.

`verified-by: bravebot_tui::app::a_picture_is_refused_in_shell_mode_rather_than_written_into_the_command`
`verified-by: bravebot_tui::app::a_picture_too_large_to_send_says_so_with_its_size`


### PASTE-7: reading the clipboard is presentation plumbing and holds no labels

Command-V is the terminal's chord and never reaches this process: the byte stream over a pty has
no encoding for that modifier, and the terminal writes the clipboard's *text* into the pty instead,
which is why a picture silently arrives as nothing. Ctrl-V comes through as a byte, so the TUI goes
around the terminal and reads the clipboard itself. An empty paste is read as the picture that was
meant. A picture wins over text when the clipboard holds both, since copying an image in a browser
leaves the page's URL behind as text and text has another key.

On macOS this reads the pasteboard through `osascript`. On Linux it needs `wl-paste` or `xclip`.

`verified-by: bravebot_tui::app::an_empty_paste_goes_and_reads_the_clipboard_instead`
`verified-by: bravebot_tui::app::a_paste_that_carried_text_is_left_alone`
`verified-by: bravebot_tui::app::which_key_carries_a_picture_is_said_once_per_session`
`verified-by: bravebot_tui::clipboard::a_missing_tool_reads_as_nothing_on_the_clipboard`
`verified-by: bravebot_tui::clipboard::a_missing_tool_is_not_taken_for_a_successful_copy`


### PASTE-8: every paste is named in the audit trail

With its type and size, so `--trace` and Ctrl-T account for the pictures as well as the words.

`verified-by: bravebot_core::policy::a_pasted_image_is_recorded_in_the_audit_trail`

### PASTE-9: a pasted picture is kept with the session and comes back on resume

The picture is part of the user's own message, so it is written into the session record alongside
the words and a resumed turn still has it. The redrawn transcript shows what the interface recorded
about the attachment rather than the bytes, because a data URI in the scrollback is not a
transcript.

**Why.** Resuming a session that turned on a screenshot, without the screenshot, would leave the
planner answering about something it can no longer see. Quarantined content is not written down at
all, and this is not an exception to that: a pasted picture was never quarantined, it is the user's
own input.

`verified-by: none`

## Known costs

- **A pasted picture lands on disk.** It is written into the session record so a resume can restore
  it (PASTE-9), which means a screenshot pasted into a session outlives the session. Deleting the
  session removes it.

- **A screenshot of a hostile page puts a stranger's words into the planner's context as though
  the user had typed them.** Nothing inspects the pixels and nothing could. What justifies it is
  that the user chose what to copy, can see on their own screen what they pasted, and is the party
  this serves. It is the cost shell mode carries, reached by another route.
