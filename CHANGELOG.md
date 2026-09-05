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
