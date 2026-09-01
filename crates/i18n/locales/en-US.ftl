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
cli-option-print = Non-interactive. Reads piped stdin as quarantined context
cli-option-trace = Print the audit trail
cli-option-help = Show this message
cli-option-version = Show the version


## What a command-line run says when it cannot start

cli-unknown-option = unknown option: { $flag }
cli-file-needs-a-path = --file requires a path
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
doctor-leo = leo
doctor-subscription =
    { $channel } subscription imported, { $unspent } of { $total } credentials unspent
doctor-confinement = confinement { $level }
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
leo-forgotten = forgot the { $channel } subscription
leo-looking = looking for a Leo subscription in Brave { $channel }
leo-found = found a { $environment } subscription: { $order }
leo-registering = registering this install as a new device
leo-stored = stored { $count } credentials in the system keychain, valid through { $expiry }
leo-browser-untouched =
    premium requests will now use them; the browser's own credentials were untouched


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
model-picker-keys = ↑↓ to choose  ·  Enter to select  ·  Esc to keep the current one
picker-current = current
picker-premium = premium
resume-heading = Resume session
resume-search-placeholder = Search…
resume-keys = ↑↓ to choose  ·  Enter to resume  ·  type to search  ·  Esc for a new session
resume-nothing-matches = nothing matches that


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
status-endpoint = Endpoint
status-premium-configured = premium configured
status-free-tier = free tier only
status-confinement = Confinement
status-this-session = This session
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
