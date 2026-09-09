---
id: UPDATE
title: Telling somebody their bravebot is out of date
status: normative
governs:
  - crates/tui/src/update.rs
  - crates/tui/src/app.rs
  - npm/bin/bravebot.js
  - install.sh
---

## Scope

One line on the startup screen: a newer version exists, and here is what installs it. What names
a version and what publishes one is [releases.md](releases.md), which also governs what an
installer checks before it writes an executable. This file governs learning, afterwards and from
a running copy, that a newer one is out.

No model is involved and no turn has begun. The request is this program's own, made on its own
behalf outside any session, and what comes back is a version number.

## Clauses

<a id="UPDATE-1"></a>
### UPDATE-1: the startup screen says what is already on disk, and waits for nothing

The line is said once, after the trust question has been answered, from an answer an earlier
launch wrote down. Asking a registry happens behind the running session, on a thread nothing
waits on, and what it learns is for a later launch.

**Why.** A network round trip before the first frame makes every session on a slow, captive or
absent connection start slowly, in exchange for an aside. Placing the line after the trust
question keeps the first thing on the screen the question that grants standing permission.

`verified-by: by-construction (the startup path reads one file, and the request is handed to a spawned thread whose handle is dropped, so nothing can wait on it)`

<a id="UPDATE-2"></a>
### UPDATE-2: only an installation there is a command for is told anything

Two are: the npm package, which its launcher names when it starts the binary, and an install made
by the install script, which is the binary that script recorded. Any other copy, one built from
source above all, is told nothing and asks nothing.

**Why.** The line exists for the command in it. Telling somebody a release is out without a line
that would install it is noise, and a build from source is updated by the tree it was built from.
The recorded path has to be the binary that is running, since a checkout on a machine that also
has a script install is not a script install: the command would replace the other copy rather
than theirs.

`verified-by: bravebot_tui::update::the_launcher_saying_npm_is_what_makes_it_an_npm_install`
`verified-by: bravebot_tui::update::the_binary_the_script_recorded_is_a_script_install`
`verified-by: bravebot_tui::update::a_binary_other_than_the_recorded_one_is_not_a_script_install`
`verified-by: bravebot_tui::update::an_installation_nothing_recorded_is_left_alone`

<a id="UPDATE-3"></a>
### UPDATE-3: the command offered updates the copy that is running

One fixed line per installation, composed from nothing: not from the answer, not from a file, and
not from the environment.

**Why.** This is a line somebody pastes into a shell, so no part of it may come from anywhere a
value arrives from. Offering the wrong one of the two is its own harm and not a smaller one: the
npm command against a script install puts a second copy on the machine and updates neither.

`verified-by: bravebot_tui::update::a_newer_version_is_named_along_with_the_command_that_installs_it`
`verified-by: bravebot_tui::update::a_script_install_is_given_the_script_again`
`verified-by: by-construction (each command is one of two literals chosen by matching on the installation, so no value composes one)`

<a id="UPDATE-4"></a>
### UPDATE-4: a version is three numbers, compared as numbers

Anything else is no answer, the `v` a tag is published under included and disregarded. Only a
version strictly greater than the one compiled in is said.

**Why.** Compared as text, 10 sorts before 9 and a release goes unannounced for as long as the
numbering stays in that decade. A prerelease is not three numbers, so a startup line never sends
anybody to a release candidate, and a registry answering with something older, which is what a
withdrawn release looks like, does not send them backwards.

`verified-by: bravebot_tui::update::a_tag_is_read_with_or_without_the_v_it_is_published_under`
`verified-by: bravebot_tui::update::a_version_that_is_not_three_numbers_is_no_answer`
`verified-by: bravebot_tui::update::a_prerelease_is_not_a_version_to_offer`
`verified-by: bravebot_tui::update::versions_are_ordered_by_number_rather_than_by_spelling`
`verified-by: bravebot_tui::update::the_version_in_hand_is_not_an_update`
`verified-by: bravebot_tui::update::an_older_published_version_says_nothing`
`verified-by: bravebot_tui::update::the_version_this_was_built_as_is_three_numbers`

<a id="UPDATE-5"></a>
### UPDATE-5: what comes back decides one line and is not quoted into it

The answer is read for three numbers. The version shown is written from those numbers, so no byte
of the response reaches the screen, and none of it reaches a conversation, a request field, or a
tool result.

**Why.** The bytes are somebody else's, and the rule that untrusted content never steers this
program applies to the one request it makes for itself. What they decide here is whether a line is
printed, which is the same standing the model listing has: fetched before any session, read for
the shape the endpoint documents, and reaching a person rather than a planner. Composing the line
from the parsed numbers is what keeps the response out of the system's own voice, which is drawn
without the margin that marks quarantined content.

`verified-by: bravebot_tui::update::a_version_is_said_as_the_numbers_read_rather_than_as_the_text_that_arrived`
`verified-by: bravebot_tui::update::the_registry_states_a_version_and_the_release_listing_states_a_tag`
`verified-by: by-construction (the request is made outside any turn, and the only value returned from it is a version of three numbers)`

<a id="UPDATE-6"></a>
### UPDATE-6: a registry is asked at most once a day

An answer stands for a day. A stamp in the future is asked again rather than waited out.

**Why.** A published version changes on the order of days, so asking on every launch is a request
to somebody else's registry per session for a number that has not moved. A clock that went
backwards, or a record somebody else wrote, must not leave this silent until the date catches up
with the file.

`verified-by: bravebot_tui::update::a_fresh_answer_is_not_asked_for_again`
`verified-by: bravebot_tui::update::an_answer_stamped_in_the_future_is_asked_again`

<a id="UPDATE-7"></a>
### UPDATE-7: a recorded answer belongs to the registry that gave it

The record says which registry answered, and it is read back only for an installation that would
ask that one.

**Why.** The two registries answer for different things, and npm has a version only once it has
been published there. On a machine where both installations have existed, reading one as the other
offers a command for a version that registry does not serve.

`verified-by: bravebot_tui::update::a_stored_answer_is_read_back_as_it_was_written`
`verified-by: bravebot_tui::update::an_answer_from_the_other_registry_is_not_read`

<a id="UPDATE-8"></a>
### UPDATE-8: a session that keeps nothing records no answer and asks for none

An [incognito](incognito.md) session still reads the answer an ordinary session left and still
says the line. It writes no answer, and it makes no request that would produce one.

**Why.** Reading is what makes that mode private rather than crippled, and a session that stopped
saying a copy was superseded would be the second. Asking without recording is a request whose
answer is thrown away on arrival.

`verified-by: bravebot_tui::update::a_session_that_records_nothing_asks_nothing`
`verified-by: bravebot_tui::incognito::the_answer_about_updating_is_still_read`
`verified-by: by-construction (the write resolves the directory through the writing answer, which is what that mode answers nothing to)`

<a id="UPDATE-9"></a>
### UPDATE-9: everything that can go wrong here is silence

No home directory, no network, a refused or unreachable registry, an answer of an unexpected
shape, a record that is not the shape written: each of them says nothing and reports nothing.

**Why.** This is an aside on a startup screen. An error from a third party's registry, printed
where somebody is about to start work, reads as this program being broken, and none of it stops a
session from doing anything.

`verified-by: bravebot_tui::update::an_answer_that_is_not_the_shape_expected_is_no_version`
`verified-by: bravebot_tui::update::a_file_that_is_not_the_shape_written_here_is_no_answer`
`verified-by: bravebot_tui::update::a_version_that_is_not_three_numbers_is_no_answer`

<a id="UPDATE-10"></a>
### UPDATE-10: the install script's record names the install and is where the next one goes

The script writes down the path it installed, and a later run with no directory named installs
over that binary.

**Why.** One record answers both questions, and it has to: an update that landed in a different
directory would leave two copies on the machine and update whichever the PATH does not reach,
while a copy the record does not name is one this program will not offer a command for.

`verified-by: by-construction (the script writes the destination path into the record after moving the binary, and takes the directory from that record when none is given)`

## Known costs

- **A first run says nothing.** With no answer on disk yet, the earliest a fresh install can be
  told about a release is its second launch. The alternative is a round trip in front of the first
  frame, which UPDATE-1 exists to refuse.

- **An answer is up to a day out of date.** A release published this morning is announced
  tomorrow to somebody whose last launch was last night.

- **What names the installation is a word and a path, not proof.** A binary moved out from under
  the record is told nothing, and a process started with the launcher's variable set is offered
  the npm command whoever set it. The cost is the wrong line of the two, never a wrong
  destination: both commands and both registries are literals in this program.

- **A version that is not three numbers is invisible.** A release published as `1.0`, or as a
  release candidate, is never announced, and neither is a tag with anything after the patch
  number.

- **The two registries are announced at different moments.** The npm package is published after
  the GitHub release exists, so in the window between them a script install is told about a
  version npm does not serve yet. Each installation asks the registry it came from, so neither is
  offered a command that cannot work.

- **Nothing runs the install script.** UPDATE-10 is by-construction for the reason every clause in
  [releases.md](releases.md) is: it is shell a person fetches over the network, and a test that
  ran it would install something.

- **Windows is npm only.** The install script is for macOS and Linux, so a Windows copy is told
  about a release only through the launcher that npm installs.
