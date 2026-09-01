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
