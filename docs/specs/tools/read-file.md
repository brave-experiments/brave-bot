---
id: READ
title: read_file
status: normative
governs:
  - crates/agent/src/workspace.rs
---

## Scope

Reading one file. `path`, `offset` and `limit` are routing; there are no content arguments. The
result is the lines, or a reference when the planner may not see them.

## Clauses

<a id="READ-1"></a>
### READ-1: a read the planner may not see does not open the file

Where the result would be quarantined, the path is checked, the file is confirmed present and
text, and the label is fixed from the trust map, all at the moment the planner asks. The bytes are
read only when a processor or a write needs them, and the path is checked **again** then, so a
file that lost its trust in between is read at the lower label.

**Why.** Most of what an agent reads in a directory nobody vouched for is a file it turns out not
to want. Re-checking on the second pass is what stops a reference issued at one label delivering
bytes at another.

`verified-by: bravebot_agent::turn::a_file_the_planner_may_not_see_is_reserved_rather_than_opened`
`verified-by: bravebot_agent::turn::a_page_of_a_file_the_planner_may_not_see_is_reserved_too`
`verified-by: bravebot_core::policy::a_path_that_lost_its_trust_fills_the_slot_untrusted`
`verified-by: bravebot_core::policy::a_read_from_an_unvouched_path_is_untrusted`
`verified-by: bravebot_core::policy::a_read_from_a_trusted_path_is_trusted`

<a id="READ-2"></a>
### READ-2: a long file is paged, and says where to continue from

Reads cap at 500 lines and 2000 characters per line, report the range returned, and give the
offset to continue from.

`verified-by: bravebot_agent::workspace::a_paged_read_is_capped_and_says_where_to_continue`
`verified-by: bravebot_agent::workspace::the_reported_next_offset_returns_the_following_lines`
`verified-by: bravebot_agent::workspace::an_over_long_line_is_shortened_and_counted`
`verified-by: bravebot_agent::workspace::an_offset_past_the_end_returns_nothing_and_says_the_length`
`verified-by: bravebot_agent::turn::the_model_can_ask_for_a_later_page`

<a id="READ-3"></a>
### READ-3: a file that is not text is reported as binary

Never as a decoding error, which would read as a fault rather than as a fact about the file.

`verified-by: bravebot_agent::workspace::a_binary_file_is_reported_as_binary`
`verified-by: bravebot_agent::workspace::a_paged_read_of_a_binary_file_is_refused`
`verified-by: bravebot_agent::workspace::text_files_are_not_mistaken_for_binary`
`verified-by: bravebot_agent::workspace::an_empty_file_is_not_binary`

<a id="READ-4"></a>
### READ-4: the planner may choose which file to read

A read changes nothing and is confined to the working directory, so the choice is promoted rather
than put to a person, and every such choice is recorded as a promotion so an audit can separate
the planner's decisions from the user's.

`verified-by: bravebot_core::policy::a_model_proposal_can_be_promoted_for_a_confined_read`
`verified-by: bravebot_core::policy::a_read_and_a_write_leave_different_trails`

<a id="READ-5"></a>
### READ-5: what a slot holds is the file, not a reader's view of it

The caps in READ-2 bound what reaches a context or a screen. They do not apply to the copy taken
for a processor or a write, which is read whole, byte for byte, with no line dropped, no long line
shortened, and no note about either appended.

**Why.** A slot's contents are written back over the file they came from, so every shaping the
pager does for a reader is destruction here. Nobody is placed to catch it: the planner may not read
the file, the processor is handed the shortened copy as though it were the whole, and what comes
back replaces the original. The two reads exist for different audiences and only one of them has a
budget to keep.

`verified-by: bravebot_agent::workspace::the_whole_of_a_file_is_read_without_the_pagers_shaping`
`verified-by: bravebot_agent::workspace::a_whole_read_keeps_the_files_own_ending`
`verified-by: bravebot_agent::workspace::a_whole_read_of_a_binary_file_is_refused`
`verified-by: bravebot_agent::turn::a_long_quarantined_file_reaches_a_processor_whole`
`verified-by: bravebot_agent::turn::a_quarantined_file_with_a_long_line_reaches_a_processor_intact`
