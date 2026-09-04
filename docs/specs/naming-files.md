---
id: NAME
title: Naming a file with `@`
status: normative
governs:
  - crates/tui/src/entries.rs
  - crates/tui/src/state.rs
  - crates/cli/src/main.rs
---

## Scope

Writing `@src/main.rs` in a prompt, and `--file` on the command line: what each puts into the turn
and what it vouches for.

Three gestures put content into a turn on the user's own footing, and each has its own spec:
[naming-files.md](naming-files.md) for `@` in a prompt, [pasting.md](pasting.md) for Ctrl-V, and
[dropping.md](dropping.md) for a file dragged onto the window.

## Why a named file is trusted

Because the user typed the path and sending the line is the grant. Nothing a model said chose the
file, and nothing inspects the contents.

Trusted is the point here rather than a detail. The planner may read those contents, compare them,
and act on what they say, which is exactly what a read of a file nobody vouched for withholds. So
name a file when you want it worked on.

## Clauses

<a id="NAME-1"></a>
### NAME-1: a named file's contents enter the turn as trusted input

`@path` in a prompt and `--file` on the command line do the same thing and are trusted for the same
reason.

`verified-by: bravebot_agent::turn::naming_one_file_leaves_the_rest_of_the_workspace_quarantined`

<a id="NAME-2"></a>
### NAME-2: the rule is for that file, and nothing beside it

A rule on a file is more specific than any rule on the tree around it, so `@vendor/lib.js` is
trusted inside a `vendor` marked untrusted, and the rest of that directory stays exactly as it was.

`verified-by: bravebot_core::policy::naming_a_file_vouches_for_nothing_beside_it`
`verified-by: bravebot_core::policy::a_named_file_is_trusted_inside_an_untrusted_tree`

<a id="NAME-3"></a>
### NAME-3: the rule outlives the read

The file can be edited afterwards, which is usually the point of naming it, and the rule says a
person stood behind it rather than a checker having cleared it.

`verified-by: none`

<a id="NAME-4"></a>
### NAME-4: typing `@` offers what is in the workspace, so the choice is informed

The list opens on the root with directories first, a prefix narrows it, a slash descends, and
version-control and build directories are not offered. Tab completes without disturbing the rest of
the sentence, a directory completes so typing can continue into it, and the arrows and Enter choose
among what is offered.

**Why.** A path typed blind is a path the user did not really choose.

`verified-by: bravebot_tui::references::an_at_sign_offers_the_workspace`
`verified-by: bravebot_tui::references::tab_completes_a_reference_without_disturbing_the_sentence`
`verified-by: bravebot_tui::references::a_name_holding_an_at_sign_completes_to_itself`
`verified-by: bravebot_tui::references::a_directory_completes_so_typing_can_continue_into_it`
`verified-by: bravebot_tui::references::the_arrows_and_enter_choose_among_the_offered_files`
`verified-by: bravebot_tui::references::a_finished_reference_closes_the_list`
`verified-by: bravebot_tui::entries::an_empty_reference_lists_the_root_with_directories_first`
`verified-by: bravebot_tui::entries::a_prefix_narrows_the_list`
`verified-by: bravebot_tui::entries::a_slash_lists_what_is_inside_that_directory`
`verified-by: bravebot_tui::entries::noise_directories_are_not_offered`

<a id="NAME-5"></a>
### NAME-5: a name cannot leave the workspace

`..` and an absolute path are refused rather than resolved, so a named file is always inside the
working directory or a directory opened for the session.

`verified-by: bravebot_tui::references::a_reference_cannot_climb_out_of_the_workspace`
`verified-by: bravebot_tui::entries::a_reference_cannot_climb_out_of_the_workspace`

<a id="NAME-6"></a>
### NAME-6: a directory names nothing, and neither does prose

A directory is somewhere to type through rather than a file to read, so naming one includes
nothing. An address inside a sentence is not a reference, and a bare `@` names nothing.

**Why.** Including a file because somebody wrote something that looked like a path would put
content into the turn that no gesture chose.

`verified-by: bravebot_tui::references::a_directory_reference_is_not_included_as_a_file`
`verified-by: bravebot_tui::references::an_address_in_a_sentence_is_not_a_reference`
`verified-by: bravebot_tui::entries::a_directory_is_not_collected_as_a_file`
`verified-by: bravebot_tui::entries::a_bare_at_sign_names_nothing`
`verified-by: bravebot_tui::entries::what_counts_as_a_reference_being_typed`
`verified-by: bravebot_tui::entries::every_referenced_file_is_collected`

<a id="NAME-7"></a>
### NAME-7: sending finishes a half-typed name

Enter sends a prompt ending in a finished reference, and completes one ending in a half-typed
reference rather than sending the fragment.

`verified-by: bravebot_tui::references::enter_sends_a_prompt_that_ends_in_a_finished_reference`
`verified-by: bravebot_tui::references::enter_completes_a_prompt_that_ends_in_a_half_typed_reference`
`verified-by: bravebot_tui::references::enter_sends_a_finished_reference_a_directory_shares_a_prefix_with`
`verified-by: bravebot_tui::references::the_arrows_still_choose_a_row_over_a_finished_reference`
`verified-by: bravebot_tui::references::the_files_a_submitted_line_would_include`
`verified-by: bravebot_tui::references::a_cursor_past_the_end_of_a_narrowed_list_still_names_a_file`

## Known costs

- **Content you have not read is content you are vouching for.** Be as careful naming a file as
  answering yes to a directory. The planner will act on what it says.
