---
id: SESSION
title: Sessions and history
status: normative
governs:
  - crates/tui/src/sessions.rs
  - crates/tui/src/state.rs
  - crates/tui/src/history.rs
  - crates/tui/src/store.rs
  - crates/cli/src/main.rs
---

## Scope

What is kept between runs: the record of a session so it can be picked up again, the prompts a
person typed so they can be recalled, and what a session says on its way out about being picked up.
What a resume does to standing permissions is [prompting.md](prompting.md); what the trail contains
is [trace.md](trace.md). Everything else the command line does is [cli.md](cli.md), which governs
the same file for its own topic.

## Clauses

<a id="SESSION-1"></a>
### SESSION-1: a session belongs to the directory it ran in

Records live under one directory per working directory, so the list worth seeing when resuming in
one project is not the list from another. Each session is two files: the record, holding what the
picker shows and what a resume needs, and the trail, appended a turn at a time.

A session is named by a version 4 UUID, and the two files are named after it. Random rather than
counted or clocked, so two sessions cannot collide however many are running, and opaque because the
name is printed on a screen and pasted into a command: it used to be the time and the process id,
which is two facts about the machine that are nobody's business by then. Nothing orders sessions by
it; the list is sorted on what each record says it was last written.

`verified-by: bravebot_tui::sessions::sessions_are_written_read_back_and_kept_per_directory`
`verified-by: bravebot_tui::sessions::a_session_is_named_by_a_uuid`
`verified-by: bravebot_tui::sessions::no_two_sessions_are_given_the_same_name`
`verified-by: bravebot_tui::sessions::a_list_puts_the_most_recently_written_session_first`
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

Each is stored with when it was sent and which workspace it was sent from, both of which the search
over the history reads ([terminal-input.md](terminal-input.md#INPUT-20)). A line written before
either was kept is read as a prompt with neither, rather than being dropped or given an invented
time, and is written back out the way it came in.

**Why.** Neither fact can be worked out afterwards: a file's own timestamp says when the newest
prompt was added and nothing about the rest, and a prompt's workspace is gone the moment the session
that sent it ends. Somebody's history is also the one file here whose loss they would notice, so a
format that could not read the previous one would be paid for in exactly the thing this is for.

`verified-by: bravebot_tui::persist::a_prompt_sent_now_is_stored_for_next_time`
`verified-by: bravebot_tui::persist::when_and_where_a_prompt_was_sent_outlive_the_session`
`verified-by: bravebot_tui::persist::a_history_from_an_older_version_is_still_read`
`verified-by: bravebot_tui::state::a_sent_prompt_records_when_and_where_it_was_sent`
`verified-by: bravebot_tui::store::when_and_where_a_prompt_was_sent_survive_a_round_trip`
`verified-by: bravebot_tui::store::a_line_from_an_older_history_is_still_a_prompt`
`verified-by: bravebot_tui::store::a_prompt_with_no_stamp_is_not_given_one_on_the_way_out`
`verified-by: bravebot_tui::store::a_prompt_holding_tabs_is_still_one_prompt`
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

<a id="SESSION-8"></a>
### SESSION-8: a session says how to pick it up again as it ends

Leaving prints the command that resumes this session, and the id is the one that fetches it. It is
printed after the terminal is handed back, so it stays on the screen the person is left looking at
rather than going with the interface. A session that never wrote a record prints nothing.

**Why.** A session is worth resuming far more often than anybody thinks to write its name down
beforehand, and the picker is no use to someone who has already closed the window. Naming a
session with no record behind it would be worse than saying nothing: the command would answer "no
session by that name".

`verified-by: bravebot_tui::sessions::a_session_is_named_once_there_is_a_record_to_name`

<a id="SESSION-9"></a>
### SESSION-9: the theme name is stored globally, like the model

Which theme paints the interface is written under `~/.bravebot` and read back at the next start.
It is not a property of a checkout: the same choice applies in every directory, and an empty or
corrupt file is no choice at all, falling back to `brave`. Custom theme files live beside it, under
`~/.bravebot/themes`, which [terminal-transcript.md](terminal-transcript.md) governs.

**Why.** Asking again in every project for the same preference is answering it repeatedly, and
nothing about a theme depends on which files are open.

`verified-by: bravebot_tui::persist::a_chosen_theme_is_read_back_next_session`
`verified-by: bravebot_tui::store::a_stored_theme_is_read_back_without_its_newline`
`verified-by: bravebot_tui::store::an_empty_theme_file_is_not_a_choice`
`verified-by: bravebot_tui::store::only_the_first_theme_line_is_read`
`verified-by: bravebot_tui::store::an_over_long_theme_name_is_not_a_choice`

<a id="SESSION-10"></a>
### SESSION-10: a manifest run is recorded, and cannot be continued

The goal, the proposed plan, the frozen steps, and what each one did are written into the record,
finished or not. The conversation is empty: a session is turns over one conversation, and a
manifest run has none. The picker marks the row and refuses Enter rather than loading an empty
session and asking the model to carry on from nothing. Naming one on the command line prints
what it produced, and still does not continue it.

`verified-by: bravebot_tui::sessions::a_manifest_run_is_recorded_and_cannot_be_resumed`
`verified-by: bravebot_tui::resume::a_manifest_session_cannot_be_resumed`
`verified-by: bravebot_tui::resume::a_manifest_run_is_marked_in_the_list`

<a id="SESSION-11"></a>
### SESSION-11: the record says what answered, and what each turn cost

The model the server reported answering with is written down, along with what each turn spent as
well as the total. The breakdown adds up to the total, and a turn that compacted part way through
is charged for that too, since it was asked for in the middle of that turn's work.

The name recorded is the one that answered, not the one asked for: an endpoint may serve something
other than the name it was given, and the record is an account of what happened. A record written
before either was kept reads as no model and an empty breakdown, which is not the same as a
session that cost nothing: the total is still there.

**Why.** A transcript is read after the fact to find out why a session went the way it did, and
both questions are unanswerable from a total alone. Twenty even turns and one turn that ran away
come to the same figure and want different fixes. Two sessions cannot be compared at all without
knowing which model produced each, and a global setting read afterwards is today's answer rather
than the one in force at the time.

`verified-by: bravebot_tui::sessions::sessions_are_written_read_back_and_kept_per_directory`
`verified-by: bravebot_tui::state::each_turn_records_what_it_cost_on_its_own`
`verified-by: bravebot_tui::state::an_aside_is_charged_to_the_turn_it_interrupted`
`verified-by: bravebot_tui::state::clearing_forgets_what_each_turn_cost`

<a id="SESSION-12"></a>
### SESSION-12: the record says where each turn's time went, not only how long it took

Every turn's wall clock is written down split four ways: what was spent waiting on the model, what
was spent running tools, what was spent waiting for the person to answer a prompt, and what is left
over. The four are a partition rather than four independent measures, so the parts account for the
whole and the remainder is meaningful. An approval prompt is drawn from inside a tool call, so what
was spent waiting for a person is taken off the tool figure rather than counted in both.

A turn that failed is recorded on the same footing as one that succeeded, and a `/compact` asked for
mid-turn is charged to the turn it interrupted, as its tokens are. `/status` reports the session
total and each part that actually happened; a part that did not happen is left out rather than shown
as zero. A record written before this was kept reads as an empty breakdown, which is not the same as
a session that took no time.

**Why.** A duration alone is unactionable, and the three things it conflates want three different
fixes. A turn that took four minutes on the model, one that took four minutes running a test suite,
and one that took four minutes with a diff on the screen while its user was at lunch are the same
number. Only the last is not the machine's fault, and it is the one a total can never reveal:
stalled time was previously invisible, so the harness's own overhead and a person's thinking time
were indistinguishable from inference.

`verified-by: bravebot_tui::state::each_turn_records_where_its_time_went`
`verified-by: bravebot_tui::state::an_aside_charges_its_wait_to_the_turn_it_interrupted`
`verified-by: bravebot_tui::state::a_failed_turn_still_accounts_for_its_wall_clock`
`verified-by: bravebot_tui::state::a_resumed_session_carries_on_from_the_time_it_had_spent`
`verified-by: bravebot_tui::sessions::sessions_are_written_read_back_and_kept_per_directory`
`verified-by: bravebot_tui::sessions::a_record_written_before_timing_was_kept_still_loads`
`verified-by: bravebot_tui::status::the_panel_says_where_the_session_spent_its_time`
`verified-by: bravebot_tui::status::a_part_that_never_happened_is_not_reported_as_zero`
`verified-by: bravebot_tui::status::a_session_with_no_turn_yet_reports_no_time`
`verified-by: bravebot_agent::confirm::the_time_a_person_takes_to_answer_is_counted`
`verified-by: bravebot_agent::confirm::every_kind_of_question_is_timed`
`verified-by: bravebot_agent::confirm::a_refusal_is_a_wait_like_any_other`
`verified-by: bravebot_agent::confirm::the_answer_passes_through_untouched`
`verified-by: bravebot_agent::timing::the_remainder_is_what_nothing_else_accounts_for`
`verified-by: bravebot_agent::timing::parts_exceeding_the_whole_do_not_wrap`
`verified-by: bravebot_agent::turn::time_spent_waiting_for_an_approval_is_not_charged_to_the_tool`

## Known costs

- **Two working directories can share a session store.** The directory name is derived by mapping
  every character outside a small set to `-`, which is lossy, so `/a/b`, `/a-b` and `/a b` all
  reduce to the same name. Nothing re-checks afterwards: the listing reads every record in that
  directory without comparing the path recorded inside it. Since a resume restores standing
  permissions, permissions granted in one of those directories would be offered in another.

  The record does hold the true path, so the fix is to filter on it. `two_directories_do_not_share_a_key`
  does not cover this: it compares `/a/one` with `/a/two`, which differ before the mapping is
  applied.
