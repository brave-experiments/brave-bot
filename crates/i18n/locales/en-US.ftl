# The reference catalog. It owns the set of messages and the name and kind of every
# argument, so a translation can add none of its own and change no call site.
#
# Ids are kebab-case and grouped by the surface they appear on. A message is named for
# what it says, never for where it happens to sit, so moving a line between two panels
# does not rename it.
#
# What is not here: anything the planner reads. A tool's description, the preamble, and
# the sentence a refused tool answers with are interface to a model rather than prose for
# a person, and translating them would change what the agent does.


## Counting

count-turns = { $count ->
    [one] { $count } turn
   *[other] { $count } turns
    }


## Starting up, and the words printed before an interface exists

cli-tagline = bravebot { $version }: a general-purpose agent resistant to prompt injection
cli-usage-heading = Usage:
cli-usage-interactive = Start an interactive session
cli-usage-task = Run a single task
cli-usage-piped = ...with piped input, never trusted
cli-usage-resume = Pick up a session in this directory
cli-usage-doctor = Check configuration and confinement
cli-usage-import = Import a Leo Premium subscription

cli-keys-heading = Interactive keys:
cli-key-send = Send
cli-key-audit = Toggle the audit trail
cli-key-history = Walk back through sent prompts
cli-key-scroll = Scroll the transcript
cli-key-jump = Jump to the start or the latest
cli-key-cancel = Cancel a running turn, clear the input, or leave
cli-key-leave = Leave

cli-commands-heading = Interactive commands:
cli-name-a-file = Include a workspace file as trusted context

cli-options-heading = Options:
cli-option-file = Include a workspace file as context (repeatable)
cli-option-mode = turn (default) decides step by step; manifest plans the whole run first
cli-option-print = Non-interactive. Reads piped stdin as quarantined context
cli-option-trace = Print the audit trail
cli-option-help = Show this message
cli-option-version = Show the version


## What a command-line run says when it cannot start

cli-unknown-option = unknown option: { $flag }
cli-file-needs-a-path = --file requires a path
cli-mode-needs-a-name = --mode requires one of { $names }
cli-unexpected-argument = unexpected argument: { $argument }
cli-task-required = a task is required
cli-configuration-problem = configuration error: { $problem }
cli-workspace-problem = workspace error: { $problem }
cli-interface-problem = interface error: { $problem }
cli-directory-unknown = cannot tell which directory this is
cli-no-such-session = no session { $id } in this directory
cli-piped-input-unreadable = warning: could not read piped input: { $problem }
cli-piped-input-too-large =
    piped input is larger than { $limit } MiB. Write it to a file and name that instead


## What a finished one-shot run says beside the reply

cli-notice = note: { $notice }
cli-model-used = model: { $model }
cli-something-was-refused = note: a policy gate refused something during this turn
cli-resume-heading = Resume this session with:


## Reporting configuration and confinement

doctor-configuration-ok = configuration OK
doctor-endpoint = endpoint
doctor-premium = premium
doctor-premium-absent = not configured
doctor-key-id = key id
doctor-model = model
doctor-model-chosen = { $model } (chosen with /model)
doctor-model-default = { $model } (default)
doctor-key-name = key
doctor-key = { $key } (never transmitted)
# Both are reported when both are reachable, so this names one of the two rather than the backend.
doctor-backend = offers
doctor-backend-bedrock = AWS Bedrock
doctor-backend-aichat = Brave Leo
doctor-backend-gateway = { $gateway } (gateway)
# Whether one was found, never the value: on this path it is a bearer token, and a diagnostic that
# printed one is a diagnostic people paste into issues.
doctor-gateway-token = found (never printed)
doctor-gateway-token-absent = none found (set a variable its `env` names)
doctor-gateway-models-absent = none configured (the gateway is asked what it serves)
doctor-region = region
doctor-profile = profile
doctor-profile-absent = default credentials
doctor-tiers = models
doctor-tiers-absent = none configured (set ANTHROPIC_DEFAULT_OPUS_MODEL)
doctor-settings = settings
doctor-settings-names = { $names }
doctor-settings-absent = no settings.json
doctor-permissions = permissions
doctor-permissions-absent = no rules
doctor-permissions-count =
    { $count ->
        [one] { $count } rule
       *[other] { $count } rules
    }
doctor-permissions-unreadable = unreadable rule
# A file that configures a gateway names no variables, and reporting that as an absent file would
# describe a file the person is looking at.
doctor-settings-no-variables = settings.json, naming no variables
doctor-leo = leo
doctor-subscription =
    { $environment } subscription imported, { $unspent } of { $total } credentials unspent
doctor-confinement = confinement { $level }
# How much confinement was actually achieved. The sandbox reports which of the three it got and
# the interface is what names it, because bravebot-sandbox holds no words for a person.
confinement-kernel = kernel-enforced
confinement-partial = partial
confinement-none = none
doctor-mechanisms = mechanisms
doctor-network-denial = network denial
doctor-kernel-enforced = kernel-enforced
doctor-not-enforced = NOT enforced
doctor-confinement-unavailable = confinement unavailable


## Importing a Leo Premium subscription

leo-no-premium-endpoint =
    warning: this build has no premium endpoint, so imported credentials will not be used
leo-set-and-rebuild = set { $variable } and rebuild
leo-unknown-channel = unknown channel: { $channel }
leo-expected-channel = expected one of: stable, beta, nightly, development
leo-forgotten = forgot the imported subscription
leo-looking = looking for a Leo subscription in Brave { $channel }
leo-found = found a { $environment } subscription: { $order }
leo-registering = registering this install as a new device
leo-stored = stored { $count } credentials in { $path }, valid through { $expiry }
leo-browser-untouched =
    premium requests will now use them; the browser's own credentials were untouched

# Said when a subscription is stored but could not be read. Worth a line because the request
# then goes out on the free tier, where a premium model name is answered by a weaker model
# rather than by an error, so the only symptom is a worse answer.
subscription-unusable =
    the imported subscription could not be used ({ $problem }), so this turn runs on the free tier


## Vouching for a directory, asked once when a session starts somewhere new

trust-directory-title = trust this directory?
trust-directory-question = Trust
trust-directory-explained =
    Files here will be read as trusted, and edits to them will not be shown to you one by
    one. Say no if you did not write this code.
trust-directory-regardless =
    Either way, anything derived from the web or from an untrusted file is still shown
    before it is written.
trust-directory-yes = trust it
trust-directory-no = ask me about every write
quit = quit


## Choosing a theme, a model, or a session to pick up

theme-picker-title = themes
theme-picker-keys = ↑↓ choose  ·  Enter select  ·  Esc keep current
model-picker-heading = Select model
model-picker-keys = ↑↓ choose  ·  Enter select  ·  type to search  ·  Esc keep current
model-picker-search-placeholder = Search
model-picker-nothing-matches = nothing matches that
picker-current = current
picker-premium = premium
# The heading over the models Brave's own endpoint serves. Named rather than left blank, because a
# list whose other sections name a service reads as though the unlabelled rows came from nowhere.
picker-service-brave = Brave
# Both rosters are offered at once, and a tier name alone does not say which of the two it is: the
# same model is reachable through either, billed and reached differently. Not "Bedrock" alone, which
# the Brave roster already says of the models it serves through its own account.
picker-service-bedrock-profile = Bedrock, your { $profile } AWS profile
picker-service-bedrock = Bedrock, your AWS account
resume-heading = Resume session
resume-search-placeholder = Search…
resume-keys = ↑↓ to choose  ·  Enter to resume  ·  type to search  ·  Esc for a new session
resume-nothing-matches = nothing matches that
resume-manifest-run = that was a manifest run, which cannot be continued; start a new session


## Shared by every question the interface stops to ask

stop-the-turn = stop the turn
scroll-more = ↑↓ { $count } more
scroll-back = ↑↓ back


## Approving a write

write-title = approve this write?
write-create = Create
write-overwrite = Overwrite
write-edit = Edit
write-tally = +{ $added } -{ $removed }
write-too-large-to-show =
    the change is too large to show: { $added } lines replace { $removed }
write-untrusted = untrusted: nobody has read this, and the model never saw it
write-unchanged = { $count ->
    [one] … { $count } unchanged line
   *[other] … { $count } unchanged lines
    }
write-yes = write it
write-no = leave it alone


## Approving a command

run-title = run this?
run-verb = Run
run-stages = { $count ->
    [one] { $count } stage
   *[other] { $count } stages
    }
run-in-directory = in { $directory }
run-not-sandboxed = this is not sandboxed: it runs with the access your own shell has
run-releases-private = it is also being fed your own data, which leaves here with it
run-always-explained = a: trust this exact command for the rest of this session
run-always-means-both = which means both:
run-always-runs-again = it runs again unasked, side effects and all
run-always-output-trusted = what it prints is trusted, and the model reads it
run-always-exact-arguments = these arguments only: git log would not cover git push
run-private-not-remembered =
    private input is asked about every time, so this one cannot be remembered
run-yes = run it
run-always = always
run-no = don't


## Letting the model read what a command printed

output-title = let the model read this?
output-verb = Read
output-lines = { $count ->
    [one] { $count } line
   *[other] { $count } lines
    }
output-printed-by = printed by { $command }
output-unseen =
    the model has not seen this. Approving puts it in its context, and it will act on it.
output-empty = (it printed nothing)
output-yes = let it read this
output-no = keep it back


## Vouching for a quarantined file

vouch-title = let the model read this file?
vouch-verb = Trust
vouch-explained =
    the model cannot read this file, so it is working blind on it. Vouching lets it read
    this file for the rest of this session, here and in every later read.
vouch-yes = trust it
vouch-no = leave it quarantined


## Counting the things a session accumulates

count-rules = { $count ->
    [one] { $count } rule
   *[other] { $count } rules
    }
count-commands = { $count ->
    [one] { $count } command
   *[other] { $count } commands
    }
count-tokens = { $count ->
    [one] { $count } token
   *[other] { $count } tokens
    }
# Thousands, already rounded to one place, because the exact figure is not the point at that size.
count-tokens-thousands = { $thousands }k tokens
# What separates a whole number from its fraction. English writes a point and much of Europe a
# comma, and the interface has one number in it that has a fraction at all.
number-decimal-separator = .


## What /status reports about the session

status-session = Session
status-session-untitled = untitled, nothing sent yet
status-session-id = Session id
status-directory = Directory
status-directory-trusted = trusted
status-directory-untrusted = not trusted, so every write is shown to you
status-also-open = Also open
status-added-directory = added with /add-dir
status-model = Model
status-model-chosen = chosen with /model
status-model-default = the configured default
status-theme = Theme
status-theme-chosen = chosen with /theme
status-served = Answered by
status-served-instead = served instead of the model asked for
status-endpoint = Endpoint
# Which tier the last turn actually ran on, not what this build was compiled knowing about.
status-premium-available = premium available, nothing sent yet
status-premium-in-use = premium, a credential was spent
status-premium-not-spent = free tier: no subscription was used
status-free-tier = free tier only
status-confinement = Confinement
status-loop = Loop
status-loop-every = every { $every }
status-loop-self-paced = paced by each turn
status-loop-next = next in { $next }
status-loop-running = running now
status-loop-unpaced = waiting for the turn to say when
status-this-session = This session
# Where a session's wall clock went. Four figures, because the whole is unactionable: a session
# that took an hour on the model, an hour on subprocesses, and an hour waiting for its user to
# answer a prompt are three different problems with the same total.
status-time = Time
status-time-inference = on the model
status-time-tools = running tools
status-time-stalled = waiting on you
status-time-overhead = unaccounted for
status-trust = Trust
status-nothing-vouched-for = nothing vouched for
status-trusted = trusted
status-untrusted = untrusted
status-programs = Programs
status-every-run-is-asked = every run is put to you
status-trusted-commands = Trusted commands
status-trusted-commands-note = run unasked, and their output is trusted
status-and-more = … and { $count } more

# Which deployment is being talked to. Left as they are where a language borrows the English
# abbreviation, which is common for these four.
environment-local = local
environment-dev = dev
environment-prod = prod
environment-custom = custom


## The indicator drawn while a turn runs

# How long the turn has been going. No hours: a turn that ran that long has gone wrong, and
# `73m 04s` says so more plainly than `1h 13m`.
elapsed-seconds = { $seconds }s
elapsed-minutes = { $minutes }m { $seconds }s
# Beside a figure already labelled in tokens, so the unit is not repeated.
indicator-tokens-read = ↓ { $tokens } tokens
indicator-tokens-written = ↑ { $tokens }
# Abbreviated counts, already rounded to one place.
tokens-thousands = { $thousands }k
tokens-millions = { $millions }M
# Said once a turn is over, because the end of one used to be announced by the indicator
# disappearing, and an announcement made by something vanishing is one nobody reads.
turn-done = turn { $turn } done
turn-failed = turn { $turn } stopped


## Picking up a session that ran somewhere, or on something, else

session-reopen-failed = could not reopen { $directory }: { $problem }
session-branch-moved = this session ran on { $was }; this checkout is on { $now }
session-branch-gone = this session ran on { $was }; this checkout is not on a branch
session-branch-new = this session ran on no branch; this checkout is on { $now }
session-build-differs = that session ran on bravebot { $was }; this is { $now }


## Themes

theme-follows-terminal = follows your terminal, light or dark


## Answering a question the agent asked

ask-title = the agent is asking
ask-title-numbered = the agent is asking ({ $at } of { $total })
ask-own-words = Answer in my own words
ask-more-options = … { $count } more, use the arrow keys
ask-key-move = move
ask-key-pick-any = pick any
ask-key-pick-one = pick
ask-key-answer = answer
ask-key-skip = skip
ask-key-skip-question = skip the question
ask-key-back-to-options = back to the options


## Handing the line to an editor

editor-none-configured = no editor found: set $VISUAL or $EDITOR to the one you want
editor-scratch-unusable = the file to edit could not be used: { $problem }
editor-named-but-missing =
    '{ $command }' was not found, and $VISUAL or $EDITOR names it, so nothing else was tried
editor-exited-badly = { $editor } exited with status { $code }, so the line is unchanged
editor-was-stopped = { $editor } was stopped before it finished, so the line is unchanged
editor-would-not-start = { $editor } would not start: { $problem }


## The transcript

input-placeholder = Ask Brave Bot to do anything
quarantined-heading = untrusted · { $origin } · { $label }
transcript-more-lines = { $count ->
    [one] … { $count } more line
   *[other] … { $count } more lines
    }
transcript-unchanged = { $count ->
    [one] … { $count } unchanged line
   *[other] … { $count } unchanged lines
    }


## Reading back through the transcript

scroller-title = scroller
scroller-key-line = line up/down
scroller-key-half-page = half page
scroller-key-full-page = full page   (also ctrl-f / ctrl-b)
scroller-key-ends = top / bottom   (also home / end)
scroller-key-prompts = previous / next prompt
scroller-key-search = search, next/previous match
scroller-key-editor = open the transcript in $EDITOR
scroller-key-this-list = this list
scroller-key-close = close the scroller
scroller-searching = enter to search  ·  esc to abandon
scroller-no-matches = no matches
scroller-match-of = { $at } of { $total }
scroller-search-keys = n next  ·  N previous  ·  esc clears  ·  q closes
scroller-rows-below = { $count ->
    [one] { $count } row below
   *[other] { $count } rows below
    }
scroller-footer = scroller
scroller-footer-keys = q closes  ·  ? keys
scroller-footer-search = / search


## The commands a line beginning with a slash may be

command-status = Report this session, what it may touch, and what it has spent
command-model = Choose which model to think with
command-theme = Choose which theme paints the interface
command-add-dir = Open another directory, and trust it for this session
command-rename = Call this conversation something else
command-compact = Summarise the conversation so far, keeping the recent part
command-clear = Start a new session here, keeping this one resumable
command-loop = Send a prompt again and again, on your interval or at a pace each turn sets
command-exit = Leave


## What the session says back

session-resumed = resumed session: { $title }
session-renamed = renamed to { $title }
session-rename-needs-a-name = /rename needs a name, as in /rename the parser bug
session-rename-needs-something = /rename needs a name with something in it
session-cleared = cleared: a new session, with the previous one still resumable
session-add-dir-needs-a-path = /add-dir needs a directory, as in /add-dir ~/notes
session-directory-added = added { $directory }, and trusting it for this session
session-permission-rule-ignored = ignoring a permission rule in settings.json: { $problem }
session-directory-not-added = could not add { $directory }: { $problem }
session-using-model = using { $model }
# The picker row that said which service answers is gone by the time this is read, and the same
# name reached through two services is two bills and two credentials.
session-using-model-from = using { $model } from { $service }
# Said before the screen is handed to the AWS CLI, so a terminal filling with its output, and a
# browser opening, are accounted for rather than looking like something having gone wrong.
session-signing-in = signing in to AWS; follow the instructions below, and this returns when it is done
session-context-budget = compacting above { $budget } tokens, as this model advertises
session-models-unavailable = could not list models: { $problem }
session-theme-set = theme { $theme }
session-no-such-theme = no theme named { $theme }; try /theme for the list
session-trusting = trusting { $directory }
session-trusting-as-left = trusting { $directory } (as this session left it)
session-not-trusting = this directory is not trusted; every write will be shown to you
session-vouched-for = trusting { $path } for this session
session-answered-already = answered already: { $question }
session-something-was-refused = a policy gate refused something during that turn
# The endpoint substitutes a model it will not serve rather than refusing, so without this a
# session can ask for one model and be answered by another with nothing said.
session-model-substituted =
    { $asked } was not served: the endpoint answered with { $served }. Run `bravebot doctor` if a
    subscription was expected.
session-error = error: { $problem }
session-no-output = no output


## Repeating a prompt

loop-needs-a-prompt =
    /loop needs something to repeat, as in /loop 5m check the deploy, or /loop watch the build to
    let each turn say when to run again
loop-started-every = repeating every { $every }; ctrl-c stops it, and so does leaving
loop-started-self-paced =
    repeating at a pace each turn sets; ctrl-c stops it, and so does leaving
loop-interval-raised = the interval was raised to { $every }, which is as fast as a loop goes
loop-interval-capped = the interval was capped at { $every }, which is as long as a loop lives
loop-replaced = the loop that was running has been replaced
loop-tick = loop { $count }
loop-tick-quiet = { $quiet ->
    [one] loop { $count }, after { $quiet } tick that found nothing
   *[other] loop { $count }, after { $quiet } ticks that found nothing
    }
loop-stopped = the loop is stopped
loop-aged-out = the loop has run for a week and stopped itself
loop-unpaced = that turn did not say when to run again, so the loop has stopped
loop-busy = /loop starts with a turn of its own, so it waits until this one is done


## Pasting, dropping and attaching

paste-arrived-empty =
    that paste arrived empty: the terminal hands over text only, so a picture needs ctrl-v
paste-not-a-command = a picture is not a command: leave shell mode to paste one
paste-too-large = that picture is { $size }, and a paste carries at most { $limit }
paste-nothing-on-clipboard = there is nothing on the clipboard to paste
paste-folded = { $lines ->
    [one] [Pasted text #{ $number } +{ $lines } line]
   *[other] [Pasted text #{ $number } +{ $lines } lines]
    }
megabytes = { $size } MB


## Running a command the person typed

command-thread-stopped = the command's thread stopped unexpectedly
command-reported-a-failure = the command reported a failure


## Shortening a long conversation

compact-uninterruptible = summarising cannot be interrupted; it takes one request
compact-ended-unexpectedly = the summary ended unexpectedly
compact-done =
    summarised { $summarised } earlier messages, keeping the last { $kept } as they are
compact-nothing-to-do = there is nothing to summarise yet
compact-failed = the conversation could not be summarised: { $problem }
turn-ended-unexpectedly = the turn ended unexpectedly


## The opening screen

opening-confinement = confinement { $level }
opening-invitation = Ask a question about this workspace.


## What a turn did, in the words a transcript line begins with

# One per tool. A word rather than the tool's own name, because a person reads the line:
# "Read(src/main.rs)" says what happened and "read_file" says what was typed.
verb-read-file = Read
verb-list-files = List
verb-search = Search
verb-write-file = Write
verb-edit-file = Update
verb-todo-write = Plan
# Named for what it is rather than for what it does: every one of these is a model with no
# tools, no memory and one round, and a person watching a line go by should not have to
# remember which of the verbs meant that.
verb-spawn-processor = Isolated processor
verb-load-skill = Skill
verb-ask-user = Ask
verb-run = Run
verb-read-output = Read output
verb-schedule-next = Schedule
verb-unknown = Tool

## Where what a call produced ended up, said at the end of the line about it
#
# Names which context, because there is more than one kind of model here and the driver is not
# one of them: the planner is the model holding the conversation, and a processor is an isolated
# model that is handed slots and nothing else. "The model" answers neither question.
landed-in-the-planner = read into the planner's context
landed-quarantined = not in the planner's context; only an isolated processor can be sent to read it
landed-reserved = read by nothing: only its name is known
reach-not-the-planner = not in the planner's context; a processor can be sent to read it
reach-no-model = in no model's context: nothing can be sent to read this
