# contrib

Tools that are useful for working on bravebot but are not part of it. Nothing here is built, shipped,
or run by CI, and nothing in `crates/` depends on any of it.

## drive_tui.py

Drives the terminal interface from a script, so the parts of it that a unit test cannot reach can
be exercised anyway.

Most of the interface is wiring: a key press becomes an `Action`, the action calls into the agent,
and what comes back is drawn. The tests under `crates/tui` cover each of those pieces on its own,
and every interface bug found so far has been in the joins between them rather than in the pieces.
`/compact` shipped once with no capability to reach the model and was refused every time it was
used; the context gauge read nothing at all for a while; a summary came back and the planner
disbelieved it. None of those is visible from `cargo test`, and all three are obvious within one
scripted session.

### Running one

A script is one step per line: how long to wait, then the keys to send.

```
# Answer the trust prompt, hold four short exchanges, then summarise them.
10   y
90   say one\r
90   say two\r
90   say three\r
90   say four\r
90   /compact\r
90   what did I ask you first\r
10   /exit\r
```

```sh
cargo build
cd /tmp/scratch
contrib/drive_tui.py session.txt --squash -- target/debug/bravebot
```

The command to run goes after `--`. `-` in place of the script file reads the script from stdin.

A step's number is a timeout rather than a sleep. While a turn runs the spinner animates several
times a second, so the stream is never quiet, and the step ends as soon as it goes still. Give
each step longer than the slowest reply you expect and the script will still run at the speed of
the model.

### Asserting against the output

Pass `--squash`, and search for text with the spaces taken out:

```sh
contrib/drive_tui.py session.txt --squash -- target/debug/bravebot | grep -o 'summarised[0-9]*earlier'
```

This is not a terminal emulator. Escape sequences are stripped rather than interpreted, so the
characters come out in the right order and the wrong shape: a sentence the interface wrapped
across two lines arrives with a box border through the middle of it. `--squash` removes the
whitespace and the box drawing so a substring check can be trusted. `--raw FILE` keeps the
untouched capture if you want to look at what really happened.

### What it needs

A backend, like any other session. Either the configured one, or a local
[aichat](https://github.com/brave/aichat) with `BRAVE_AI_CHAT_ENDPOINT` pointed at it:

```sh
BRAVE_AI_CHAT_ENDPOINT=http://127.0.0.1:8000 contrib/drive_tui.py session.txt -- target/debug/bravebot
```

`BRAVEBOT_CONTEXT_BUDGET` is worth setting low when the thing being tested is compaction, since the
default is a hundred thousand tokens and a scripted session will not reach it.

Sessions it runs are real: they are written to `~/.bravebot/sessions` like any other, and a script that
approves a write will write the file. Run it somewhere disposable.
