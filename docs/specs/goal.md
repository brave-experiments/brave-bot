---
id: GOAL
title: Working towards a condition
status: normative
governs:
  - crates/agent/src/goal.rs
  - crates/agent/src/preamble.rs
  - crates/tui/src/goals.rs
  - crates/tui/src/app.rs
  - crates/tui/src/state.rs
guards:
  - symbol: Session::start_goal
  - symbol: Session::goal_not_met
  - symbol: goal::assess
  - symbol: goal::read
---

## Scope

`/goal`: one condition a person wrote, put to a judge when a turn ends, so the session carries on
until the condition holds. What the condition is, who decides whether it holds, what the driver may
do with that decision, and what ends a goal.

Not how a turn goes about it: what a turn is told about the condition is here, and the work itself
is a turn like any other. The judge is one request over a copy of the exchange, which is the shape
[watching.md](watching.md) describes for a question asked beside the work. That a `/` line is a
command at all, and that only a key press produces one, is [commands.md](commands.md).

**A goal never gets ahead of the person.** A prompt they queued while a turn ran is sent before the
judge is asked, so the condition is judged against an exchange holding everything they have said.

## What is judged

<a id="GOAL-1"></a>
### GOAL-1: the condition is the one the person typed, for as long as the goal stands

The argument to `/goal` is settled when Enter is pressed and is the condition every check is taken
against. Nothing a turn reads, writes, says or returns can add to it, edit it or replace it, and
there is no tool by which a planner may propose one. Only another `/goal` changes it.

**Why.** The condition is what decides when this session is allowed to stop. A turn that could
write its own would be a turn that decides when it has finished, and a turn that reached content an
attacker wrote would be handing that decision to them. The person endorsed a sentence, so that
sentence is what is judged.

`verified-by: bravebot_tui::app::the_goal_command_carries_the_whole_condition`
`verified-by: bravebot_agent::goal::the_condition_is_marked_as_one_rather_than_left_on_the_end_of_the_work`

<a id="GOAL-2"></a>
### GOAL-2: setting a goal starts no work

A goal is a condition and has no prompt, so arming one sends nothing and the session stays idle.
What it keeps going is whatever the person asks for next.

**Why.** There is no line here anybody endorsed. A condition is a description of the finished
state, and turning one into a first prompt would send a request the person did not write, phrased
by this program, about work it had guessed at.

`verified-by: bravebot_tui::state::setting_a_goal_starts_no_turn`
`verified-by: bravebot_tui::app::the_goal_command_sets_a_condition_without_sending_anything`

<a id="GOAL-3"></a>
### GOAL-3: the judge is a separate request, with no tools, over a copy of the exchange

The exchange is forked, the condition is put on the end of it, and one request goes out with no
tools offered and one round. The conversation itself is not written to, so the exchange the next
turn resumes is the one that was there before the check ran. The judge is told it has no tools and
that a condition about the state of the world holds only where the exchange shows it being
observed.

**Why the exchange rather than the last reply.** What was asked, what was run and what came back
are the only evidence there is, and a reply saying the work is done is a claim about them.

**Why no tools.** A judge that could run a command would be a turn, and it would be a turn whose
job is deciding whether turns stop. The one round is what leaves nothing for a reply to steer.

`verified-by: bravebot_agent::goal::the_exchange_goes_out_with_the_condition`
`verified-by: bravebot_agent::goal::checking_leaves_the_exchange_the_length_it_was`
`verified-by: bravebot_agent::goal::the_judge_is_told_it_cannot_observe_anything_the_exchange_does_not_show`

## What a verdict is

<a id="GOAL-4"></a>
### GOAL-4: the verdict is the first line, matched whole against three words

`MET`, `NOT MET` or `IMPOSSIBLE`, alone on the first line, compared for equality ignoring ASCII
case. Everything after it is the reason. A first line that is anything else is not a verdict, and
no meaning is read out of the prose.

Equality rather than a prefix, so no ordering between the three is load-bearing and `NOT MET`
cannot be read as `MET`.

**Why not guess.** Reading a verdict out of a sentence nobody constrained would let the judge's
prose decide whether this session keeps working, and a sentence explaining that the condition is
*nearly* met would read as either answer depending on which words were searched for.

`verified-by: bravebot_agent::goal::a_first_line_of_met_is_the_condition_holding`
`verified-by: bravebot_agent::goal::a_first_line_of_not_met_carries_what_is_missing`
`verified-by: bravebot_agent::goal::a_first_line_of_impossible_carries_why`
`verified-by: bravebot_agent::goal::not_met_is_never_read_as_met`
`verified-by: bravebot_agent::goal::a_sentence_about_the_condition_is_not_a_verdict`
`verified-by: bravebot_agent::goal::a_verdict_with_nothing_after_it_still_reads`

<a id="GOAL-5"></a>
### GOAL-5: the driver reads a verdict only where the planner could have held it

The judge's answer goes past the gate that decides what may be put in front of a planner. Where it
comes back quarantined, the driver does not read it, does not act on it, and the goal ends.

**Why this clause exists at all.** Acting on a verdict is a branch, and the driver does not take
branches from untrusted bytes. What makes the ordinary case sound is that the judge is given the
exchange the planner was given, and nothing untrusted is ever in that: quarantined content is a
reference, not the bytes. So the verdict is a function of trusted input and is labelled as one.

Where an exchange has met something untrusted, that reasoning stops holding and the gate is what
notices. It is not a case that arises today, because a context only becomes untrusted by resuming
one that already was. The gate is here so that a change letting untrusted bytes into an exchange
stops goals, rather than handing whoever wrote those bytes the sentence that decides whether this
program keeps working.

`verified-by: bravebot_agent::goal::the_check_carries_what_the_exchange_had_met`
`verified-by: bravebot_agent::goal::only_a_condition_that_is_not_met_yet_carries_the_work_on`

<a id="GOAL-6"></a>
### GOAL-6: one verdict carries the work on, and every other outcome ends the goal

| What came back | What happens |
|---|---|
| the condition is not met yet | the work goes back for another turn, with the reason |
| the condition is met | the goal is over, and the reason is what the person is shown |
| the condition can never be met | the goal is over, and the reason says why |
| the first line was not a verdict | the goal is over |
| the answer may not be read | the goal is over |
| the request failed | the goal is over |

**Why the unreadable cases stop rather than retry.** A stopping condition nobody can read is not a
reason to keep a session working, and the failure this is built to have is a goal that ends early
rather than one that spends a budget on a judge that has stopped answering.

`verified-by: bravebot_agent::goal::only_a_condition_that_is_not_met_yet_carries_the_work_on`
`verified-by: bravebot_tui::state::a_verdict_against_no_goal_sends_nothing`

<a id="GOAL-7"></a>
### GOAL-7: the work goes back as the driver's own sentence, naming the condition

The prompt that carries the work on is written here, says that a stopping condition has not been
met, names the condition, and quotes the judge's reason inside it. The reason is never sent on its
own.

**Why.** A bare reason arriving as a user message reads as the person having typed it, and the
planner answers it as a fresh request. It has to know that a condition it did not choose is what is
holding the session open, or it treats the reason as the whole of the job.

`verified-by: bravebot_agent::goal::the_work_is_sent_back_with_the_condition_and_not_only_the_reason`
`verified-by: bravebot_agent::goal::work_sent_back_without_a_reason_says_nothing_about_one`
`verified-by: bravebot_tui::state::a_goal_that_is_not_met_sends_the_work_back_with_the_condition_and_the_reason`

<a id="GOAL-8"></a>
### GOAL-8: a path named in that sentence vouches for nothing

`@path` in the prompt that carries the work on is prose. No file is opened by it and nothing about
it is treated as trusted context. The same words in a line the person typed still name a file, as
they always have.

**Why.** A keystroke is the whole of what makes `@path` an endorsement: the person read the name
and pressed Enter. The sentence a goal sends back is this program's, quoting a judge, so there is
no keystroke behind an `@` inside it. Reading one would let a model open a file wearing an
endorsement nobody gave, and it would do so on a path the person never saw.

`verified-by: bravebot_tui::app::a_path_named_in_a_sentence_the_driver_wrote_vouches_for_nothing`

## What ends one

<a id="GOAL-9"></a>
### GOAL-9: a goal sends the work back ten times and then gives up

The tenth round is the last, and the reason from it is kept so a goal that has given up can still
say what it kept hearing.

**Why a bound at all.** Every round is a whole turn with the conversation re-sent, and the
conversation is longer each time. Without this, a condition nobody can satisfy is a session that
spends until somebody notices, and the person who wrote a condition slightly wrong is the one who
pays for it.

`verified-by: bravebot_tui::goals::a_goal_stops_sending_the_work_back_once_its_rounds_are_spent`
`verified-by: bravebot_tui::goals::the_reason_from_the_round_that_gave_up_is_still_kept`
`verified-by: bravebot_tui::state::a_goal_that_runs_out_of_rounds_stops_rather_than_sending_the_work_back_again`

<a id="GOAL-10"></a>
### GOAL-10: four things end a goal besides a verdict, and each of them says so

| What | When |
|---|---|
| the person asks | `/goal clear` |
| the person interrupts | Ctrl-C, read against the goal after the turn in flight and the line in the box, and before leaving; Escape and Ctrl-C both reach it during a check |
| the session moves on | `/clear`, and leaving |
| the rounds run out | [GOAL-9](#GOAL-9) |

A turn that failed is not one of them, and neither is a turn somebody stopped. Both are recorded
as failures and neither is judged: a request that never came back says nothing about whether the
work is finished, and an interrupted turn says only that the person did not want that turn. The
goal stays set, and what is judged is the next turn there is something to judge.

Ctrl-C therefore means one thing at a time: the half-written line, then the turn in flight, then
the goal, then leaving. Each is nearer than the next. A person watching a goal go somewhere wrong
stops that turn and says something else, which is the only way to steer work that keeps going, and
a key that took the condition off along with the turn would leave them retyping it every time. The
press that ends the goal is the one made with nothing running.

**A check in flight is one request and does not stop, but the goal behind it does.** A verdict that
arrives about a goal somebody has just taken off is not acted on and not reported: the session they
would be told about is not the one they are in. Without this, the key pressed while the ninth round
was being judged would leave the session, and the tenth round would go out regardless.

`verified-by: bravebot_tui::state::clearing_a_goal_says_so_and_says_nothing_when_there_was_none`
`verified-by: bravebot_tui::app::interrupting_takes_the_goal_off_before_it_leaves`
`verified-by: bravebot_tui::app::stopping_a_turn_leaves_the_goal_set`
`verified-by: bravebot_tui::app::a_turn_that_failed_leaves_the_goal_where_it_was`
`verified-by: bravebot_tui::state::clearing_the_session_takes_the_goal_off`

<a id="GOAL-11"></a>
### GOAL-11: a goal and a loop are never both running

Whichever was asked for second stands, and the one it replaced is reported as stopped.

**Why.** Both keep a session working without anybody typing, and together neither is the thing that
was asked for: the interval stops meaning anything once a tick can be held open for ten more turns,
and the condition is judged against a turn that was going to be repeated anyway.

`verified-by: bravebot_tui::state::a_goal_and_a_loop_are_never_both_running`

<a id="GOAL-12"></a>
### GOAL-12: a goal is never written down

It is not in the session record, so it is not restored by a resume and does not survive the
process.

**Why.** A condition judged against an exchange from another day would take a conversation somebody
opened to read and start working it, with nothing in the transcript to say why. The gesture that
sets a goal is the gesture that keeps it: while this session is open.

`verified-by: by-construction (the goal is a private field of the interface's session state and is not among the fields written to a session record)`

<a id="GOAL-13"></a>
### GOAL-13: what is holding the session open is on the screen

Each verdict is announced as it arrives, and `/status` says the condition and how many rounds have
gone. A session with no goal says nothing about goals.

**Why.** A turn nobody typed a prompt for is the one thing about a session that cannot be read off
the transcript, and the count is the difference between a goal that is converging and one that is
about to give up.

`verified-by: bravebot_tui::status::the_report_says_what_the_session_is_working_towards_and_how_many_rounds_are_left`
`verified-by: bravebot_tui::status::a_session_with_no_goal_does_not_mention_one`

## What a turn is told

<a id="GOAL-14"></a>
### GOAL-14: every turn under a goal is told the condition

While a goal stands, each turn's system prompt states the condition, says the turn is judged
against it once it ends, and says the turn cannot change it. The first round carries it as much as
the tenth. A turn with no goal is told nothing about one.

**Why.** The driver is the only thing that knows a goal is set. A turn that is not told works on
whatever the person's line said and is then judged against a condition it never saw, so the first
round goes somewhere unrelated and comes back with a reason about work that was never aimed at
the condition. Stating it is the difference between ten rounds converging and ten rounds of a judge
explaining the condition to a turn that cannot hear it.

**Why the system prompt rather than a message.** [GOAL-1](#GOAL-1) is what makes this the right
place: the condition is settled, so it is the same sentence every turn, and a message would leave
one copy of it in the conversation per round.

`verified-by: bravebot_agent::turn::a_turn_under_a_goal_is_told_the_condition_it_is_working_towards`
`verified-by: bravebot_agent::turn::a_turn_with_no_goal_is_told_nothing_about_a_condition`

<a id="GOAL-15"></a>
### GOAL-15: a turn is told to wait for the world inside the turn

The same words say that a condition waiting on something the session does not control is waited
for in the turn, by running `sleep` and looking again, and they name `sleep` because that is what
is available for it.

**Why.** A turn that answers so as to be sent back spends one of the rounds [GOAL-9](#GOAL-9)
bounds, a judge's reading of the whole conversation, and a whole further turn, and it spends all
of that on a condition only somebody else can satisfy. Waiting inside the turn
spends a round of tools. Ten rounds of the first is a goal that gives up minutes before the thing
it was waiting for happens, which is the ordinary outcome for a condition about a file a person
has yet to write.

**Why the mechanism is named rather than left to the planner.** The line a `run` call carries is
compiled here rather than handed to a shell, and control flow is refused: see
[command-line.md](tools/command-line.md). So a planner working out how to wait writes a loop that
does not compile, and reads the refusal as there being no way to wait at all.

`verified-by: bravebot_agent::preamble::a_turn_under_a_goal_is_told_how_to_wait_for_something_outside_the_session`

## Known costs

- **The judge reads the transcript, not the world.** It cannot run a command or open a file, so a
  condition is met when the exchange shows it being met. A turn that fixes something and never
  observes the fix is sent back for not observing it, which is usually the right answer and is
  sometimes an extra turn spent proving what a person could already see.
- **A condition the transcript can never show does not converge.** "The code is clean" has no
  observation that satisfies it, so it spends all ten rounds and gives up. Nothing here can tell
  such a condition from one that is merely not met yet, and nothing warns about it in advance.
- **A wait costs a round of tools every five minutes.** A `run` is killed at five minutes, so
  waiting for something slower than that is `sleep` and another look, repeatedly, and each look is
  a request carrying the conversation. Cheaper than being sent back, and not free: a condition
  waiting on something hours away is not what a goal is for.
- **Every round re-sends the conversation.** Ten rounds of a long session cost more than ten
  ordinary turns, because each one carries everything the last one added.
