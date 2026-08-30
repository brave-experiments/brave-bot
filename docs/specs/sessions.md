---
id: SESSION
title: Sessions and history
status: normative
governs:
  - crates/tui/src/sessions.rs
  - crates/tui/src/history.rs
  - crates/tui/src/store.rs
---

## Scope

What is kept between runs: the record of a session so it can be picked up again, and the prompts a
person typed so they can be recalled. What a resume does to standing permissions is
[prompting.md](prompting.md); what the trail contains is [trace.md](trace.md).

## Clauses

<a id="SESSION-1"></a>
### SESSION-1: a session belongs to the directory it ran in

Records live under one directory per working directory, so the list worth seeing when resuming in
one project is not the list from another. Each session is two files: the record, holding what the
picker shows and what a resume needs, and the trail, appended a turn at a time.

`verified-by: bravebot_tui::sessions::sessions_are_written_read_back_and_kept_per_directory`
`verified-by: bravebot_tui::sessions::a_working_directory_becomes_one_readable_segment`
`verified-by: bravebot_tui::sessions::a_path_with_nothing_in_it_still_names_a_directory`

<a id="SESSION-2"></a>
### SESSION-2: nothing untrusted is ever written down

Every message in the record has already been past the gate that decides what the planner may see,
so what lands on disk is what the planner was allowed to hold: no untrusted bytes, by construction
rather than by filtering. Quarantined content is not written at all, and the trail is labels and
gate names with no content in it.

**Why.** A record is read back into a later turn's context. Anything written that the planner could
not have held would enter that context on the next resume, which is the laundering route the whole
design exists to close.

`verified-by: none`

<a id="SESSION-3"></a>
### SESSION-3: the record carries what a resume needs and nothing more

The conversation, the plan each turn worked to, what the session has spent, the branch it ran on,
and the standing permissions its user granted. A session can be named, renaming rewrites the record
immediately, a chosen name survives the next turn, and an empty name is refused.

`verified-by: bravebot_tui::sessions::renaming_a_session_rewrites_the_record_immediately`
`verified-by: bravebot_tui::sessions::a_chosen_name_survives_the_next_turn`
`verified-by: bravebot_tui::sessions::a_session_can_be_named_before_it_has_a_record`
`verified-by: bravebot_tui::sessions::an_empty_name_is_refused`

<a id="SESSION-4"></a>
### SESSION-4: a title comes from the prompt, and is cut rather than mangled

The first line of what was asked. A long one is cut and says it was, and a prompt with nothing in
it still has a title.

`verified-by: bravebot_tui::sessions::a_title_is_the_first_line_of_the_prompt`
`verified-by: bravebot_tui::sessions::a_long_title_is_cut_and_says_it_was`
`verified-by: bravebot_tui::sessions::a_prompt_with_nothing_in_it_still_has_a_title`

<a id="SESSION-5"></a>
### SESSION-5: everything here degrades to doing nothing

A missing home directory, a full disk, a corrupt record, a stored time in the future: a session
that cannot be written down still runs, one that cannot be read is left out of the list, and a
corrupt history reads as no history rather than as an error.

**Why.** None of this is load bearing for correctness. Failing a turn because a convenience could
not be saved would trade something that matters for something that does not.

`verified-by: bravebot_tui::sessions::a_session_from_the_future_is_not_a_crash`
`verified-by: bravebot_tui::sessions::a_stored_time_becomes_an_age`
`verified-by: bravebot_tui::persist::a_corrupt_file_reads_as_no_history`
`verified-by: bravebot_tui::persist::no_home_directory_is_not_an_error`
`verified-by: bravebot_tui::persist::the_directory_is_created_on_first_write`

<a id="SESSION-6"></a>
### SESSION-6: a submitted prompt is remembered, and a cancelled one is not

Prompts persist across runs and are capped, consecutive duplicates collapse into one, and a prompt
that was cancelled is removed again.

`verified-by: bravebot_tui::persist::a_prompt_sent_now_is_stored_for_next_time`
`verified-by: bravebot_tui::persist::a_session_recalls_a_prompt_stored_by_an_earlier_session`
`verified-by: bravebot_tui::persist::an_appended_prompt_is_read_back_next_session`
`verified-by: bravebot_tui::persist::a_cancelled_prompt_is_removed_from_the_stored_history`
`verified-by: bravebot_tui::persist::the_stored_history_is_capped`
`verified-by: bravebot_tui::persist::a_multiline_prompt_survives_a_round_trip_on_disk`
`verified-by: bravebot_tui::persist::saving_replaces_what_was_stored`
`verified-by: bravebot_tui::history::consecutive_duplicates_are_collapsed`

<a id="SESSION-7"></a>
### SESSION-7: recalling a prompt is a mode, and leaving it restores what was being typed

Up walks backwards from the most recent and stops at the oldest, Down walks forwards again, and
leaving the newest entry puts back the half-written line. Submitting leaves the mode, and a prompt
arriving while browsing does not shift the view.

**Why.** Pressing Up out of curiosity must not destroy a line somebody was part way through
writing.

`verified-by: bravebot_tui::history::up_recalls_the_most_recent_prompt_first`
`verified-by: bravebot_tui::history::up_keeps_walking_backwards`
`verified-by: bravebot_tui::history::up_stops_at_the_oldest_entry`
`verified-by: bravebot_tui::history::down_walks_forwards_again`
`verified-by: bravebot_tui::history::down_does_nothing_when_not_browsing`
`verified-by: bravebot_tui::history::leaving_the_newest_entry_restores_the_typed_line`
`verified-by: bravebot_tui::history::submitting_leaves_browsing`
`verified-by: bravebot_tui::history::appending_while_browsing_does_not_shift_the_view`
`verified-by: bravebot_tui::history::a_new_history_is_empty_and_not_browsing`
`verified-by: bravebot_tui::history::an_empty_history_has_nothing_to_recall`
`verified-by: bravebot_tui::history::the_position_counts_from_the_oldest`
`verified-by: bravebot_tui::history::popping_removes_the_newest_entry`
`verified-by: bravebot_tui::history::popping_an_empty_history_is_harmless`

## Known costs

- **Two working directories can share a session store.** The directory name is derived by mapping
  every character outside a small set to `-`, which is lossy, so `/a/b`, `/a-b` and `/a b` all
  reduce to the same name. Nothing re-checks afterwards: the listing reads every record in that
  directory without comparing the path recorded inside it. Since a resume restores standing
  permissions, permissions granted in one of those directories would be offered in another.

  The record does hold the true path, so the fix is to filter on it. `two_directories_do_not_share_a_key`
  does not cover this: it compares `/a/one` with `/a/two`, which differ before the mapping is
  applied.
