## [0.5.0](https://github.com/brave-experiments/brave-bot/releases/tag/v0.5.0)

 - Added `fetch_url`, so a turn can read a page or an issue behind an error. The body never reaches the planner, a person approves the one URL, and a redirect off the approved host is refused.
 - Added a language server tool, so a turn can ask where a symbol is defined, what references it, what a hover says, the symbols in a file or across the workspace, an implementation, and which functions call which. The server runs with your own access once you agree to it, and its index is cached under `~/.bravebot` so a later session does not wait for it again.
 - Added vi editing of the prompt: two modes, the motions, the operators, text objects such as `ci(`, and visual mode with the selection drawn while it is being chosen. The box everybody has is unchanged unless the style is chosen.
 - Added `/config`, a panel over the transcript for a preference about the interface, starting with whether the prompt edits the way vi does.
 - Added background commands to `run`, which starts a pipeline, hands back a job name, and reads what it has printed since the last look through `job_output`, so a turn can start a server and then talk to it.
 - Added regular expressions to `search`, matched without backtracking, so a pattern is answered rather than quietly treated as a literal and reported as absent.
 - Added reading a picture: a screenshot or a scanned page is handed to a processor that can look at it, while the planner gets only a reference saying what kind of thing it is.
 - Added `.bravebot/settings.json` and `settings.local.json` beside the work, resolved per name over the global file, and `bravebot doctor` now names which file a value came from where more than one sets it.
 - Added the changed lines to what an edit reports, with the line numbers they came from, so a turn can check what it produced instead of a count of replacements.
 - Added a line telling you when a turn wrote files and ran nothing, so a diff that was never built is not mistaken for a checked one, and the turn is asked once whether any of it runs.
 - Added a nudge to a turn that has read for eight rounds without writing anything, so work that is settled is written down while there is still a turn left to change.
 - Added npm publication of `@brave/bravebot`, so `npm install -g @brave/bravebot` installs a release, with the binary verified against its published checksum.
 - Changed the model list to the subset this agent is offered, and the default to `automatic-brave-bot`. The older `automatic` still resolves to it. ([#129](https://github.com/brave-experiments/brave-bot/issues/129))
 - Changed a turn to be told where it is working: the directory, the platform, the shell, whether the tree is a checkout, and today's date, so it no longer spends two prompts running `pwd` to find out.
 - Improved the wait before a reply on Bedrock by telling the service which part of a request it has already read, rather than paying for a whole conversation again every round.
 - Improved how much a turn gets done per round: calls that do not depend on each other are asked together, and a read with no window returns the file up to a page instead of thirty lines at a time.
 - Fixed a quarantined command result not saying what would show it, which left a turn concluding the shell was a dead end and reading files one at a time. It now names reading the output this turn, vouching for the command, and reading a file directly.

## [0.4.0](https://github.com/brave-experiments/brave-bot/releases/tag/v0.4.0)

 - Added a command line to `run`, with pipes, `&&`, `||`, `;`, redirections and brace, glob and tilde expansion, compiled here rather than handed to a shell and put in front of you as a plan naming every file it would write.
 - Added shift-tab, which cycles a session between asking about every write, accepting edits, planning, and bypassing, and draws the mode in force under the prompt.
 - Added `--dangerously-skip-permissions`, which answers a write, a run, a command's output and vouching for a quarantined file without asking, while deny rules from the settings file still refuse.
 - Added `/undo`, which puts the session back where it stood before the most recent turn: the files that turn wrote, the conversation, and the turn count, spend and trust map that went with it. ([#91](https://github.com/brave-experiments/brave-bot/issues/91))
 - Added `bravebot --fork <id>`, which copies a session into one with its own id and opens it, so a second approach starts from the part of the conversation worth keeping. ([#98](https://github.com/brave-experiments/brave-bot/issues/98))
 - Added `/export`, which writes the conversation out as a markdown file under the working directory, named on the line or after the session id. ([#98](https://github.com/brave-experiments/brave-bot/issues/98))
 - Added every command a turn ran to the ctrl-l list, after the delegates, so what a program printed is readable even where the planner was kept from it.
 - Added `CLAUDE.md` and `.claude/CLAUDE.md` as places a project's instructions are read from where `AGENTS.md` is absent, with a file short enough to be nothing but a pointer followed to the document it names.
 - Changed search to reach a hundred thousand files rather than two thousand, skip vendored dependencies, and accept several patterns at once along with a case-insensitive flag.
 - Fixed session records, temporary files and audit trails under `~/.bravebot` being created with the process umask, which left whole conversations readable by anyone with an account on the machine. ([#86](https://github.com/brave-experiments/brave-bot/issues/86))
 - Fixed switching to a model the endpoint does not describe raising the context budget back to the default, which left it above the window actually in force so compaction never ran.
 - Fixed the Windows builds, which failed to compile the credential store; saving a credential there is refused rather than done without the file protection Unix gets. ([#115](https://github.com/brave-experiments/brave-bot/issues/115))
 - Fixed Escape and ctrl-c cancelling the turn behind the delegate view or the prompt search instead of closing the view they were pressed in.
 - Fixed the context reading on the hint line going blank after a resume, a compaction or a failed turn, and marked a reading as approximate where the budget is one no model advertised. ([#69](https://github.com/brave-experiments/brave-bot/issues/69))
 - Fixed brace groups in a search pattern being matched literally rather than expanded, which returned no matches in the same words as a search that read the whole tree and found nothing.

## [0.3.0](https://github.com/brave-experiments/brave-bot/releases/tag/v0.3.0)

 - Added ctrl-l, which opens the list of delegates a session has run, so you can watch one working or read what it did afterwards.
 - Added a block under each delegate holding the last few things it did and the report it ends with, so work a delegate was sent off to do is readable where it was started.
 - Added `/effort`, which picks how hard a model thinks from low, medium, high, xhigh and max, keeps the choice beside the model and the theme, and sends no level to a model whose listing says it does not read one. ([#109](https://github.com/brave-experiments/brave-bot/issues/109))
 - Added `bravebot --incognito`, a session that writes nothing to `~/.bravebot`: no prompt history, no session record, no title, and no audit trail.
 - Added a pair of colours to a theme file, `{"dark": ..., "light": ...}`, resolving to the arm matching the terminal background sensed at startup.
 - Changed delegates to run alongside the turn and each other, so a turn waits for the slowest piece of work rather than the sum of it, and one call can start up to eight of them.
 - Changed `catppuccin`, `gruvbox` and `solarized` to one row each in the theme picker, painted from the half matching the terminal background, with the six fixed halves still reachable through `/theme`.
 - Changed where the confinement is reported, from a row on every frame to the mark printed at startup and `/status` on request.
 - Fixed an answer given to a prompt inside a delegate overwriting the session's whole record of what you had vouched for, which put back rules that later answers had replaced.
 - Fixed a delegate that could not finish being reported as having answered.
 - Fixed a build with no Brave credentials refusing to load its configuration when only a gateway was configured, though the gateway uses its own key. ([#106](https://github.com/brave-experiments/brave-bot/issues/106))
 - Fixed a model that writes its reasoning in `<think>` tags having that working drawn above every reply, kept in the session record and drawn again on every resume.
 - Fixed a Bedrock session that stopped working before its stated expiry going on being treated as good, which left every turn falling through to a sign-in opened where nobody could see it.
 - Fixed Bedrock attempting a sign-in for an AWS profile that is not configured, which is now reported as missing along with the profiles that exist.
 - Fixed ctrl-t being offered from the first frame, before any turn had left a trail for it to show.
 - Fixed an aside being drawn in bright black, which most terminal colour schemes leave too dim to read.

## [0.2.0](https://github.com/brave-experiments/brave-bot/releases/tag/v0.2.0)

 - Added support for an OpenAI-compatible gateway, named by a `provider` block in `~/.bravebot/settings.json` in opencode's shape, whose models are offered beside the Brave and Bedrock ones.
 - Added `/loop`, which repeats a prompt on an interval you give it or at a pace each turn sets, until ctrl-c stops it.
 - Added `/cd`, which moves a session to another directory and carries its trusted paths with it.
 - Added ctrl-r, which searches the prompts already sent and puts the one you pick in the box rather than sending it.
 - Added the `permissions` block from Claude Code's settings file, so allow, ask and deny rules and `additionalDirectories` copied out of `~/.claude/settings.json` govern this agent unedited.
 - Added the `model` key in `~/.bravebot/settings.json`, where `opus`, `sonnet` and `haiku` name a tier and resolve to a model a reachable service serves.
 - Added search to the model list, which now filters as you type and is grouped by the service that answers rather than drawn as one flat list.
 - Added a breakdown to `/status` of where a session's time went: the model, tools, waiting for you, and the rest.
 - Changed the Bedrock environment variable to `BRAVEBOT_USE_BEDROCK`. The old name is no longer read.
 - Changed where an imported Brave subscription is kept, from the system keychain to one file only you can read, so a machine with no desktop session can use it. Import it again to move an existing one, and `--forget` no longer takes a channel.
 - Changed a prompt typed while a turn is running to reach that turn between rounds, instead of waiting until the whole turn has finished. ([#68](https://github.com/brave-experiments/brave-bot/issues/68))
 - Fixed `run` handing this agent's own signing credentials to every program it starts, which no approval ever showed. ([#84](https://github.com/brave-experiments/brave-bot/issues/84))
 - Fixed the delay before every turn on Bedrock, caused by re-checking the AWS session each time.
 - Fixed the confirm prompts and the trust prompt drawing their contents in the terminal's own colours instead of the theme's, which made them unreadable under a light theme in a dark terminal.
 - Fixed the prompt being drawn after the checks that run before a turn, which left it blank for a moment.
 - Fixed `?`, ctrl-t and ctrl-g being ignored while a turn was running, and the key list being left standing over a line written under it. ([#66](https://github.com/brave-experiments/brave-bot/issues/66))
