#!/usr/bin/env python3
"""Drive the bravebot TUI from a script, for testing the paths a unit test cannot reach.

The interface is most of this program and none of it is reachable from `cargo test`. A slash
command, the context gauge, a trust prompt, resuming a session: each one is a keystroke going
into a real terminal and pixels coming back, and the tests underneath them can only check the
pieces. Every bug found in the TUI so far has been in the wiring between those pieces.

A pty is what makes this possible. The interface reads keys from a terminal rather than from
standard input, so piping to it does nothing: the process has to be given a terminal of its own,
which is what `pty.fork` does here.

Waiting is by silence rather than by clock. While a turn runs the spinner animates several times
a second, so the stream is never quiet; the moment it stops, the turn is over. So each step names
the longest it will wait and returns as soon as the screen has been still for a moment, which is
both quicker and far less brittle than sleeping for a guess.

What this is not is a terminal emulator. Escape sequences are stripped rather than interpreted,
so what comes out has the right characters in the right order and the wrong shape: a sentence the
interface wrapped across two lines arrives with a box border in the middle of it. Assert against
`--squash`, which removes whitespace and box-drawing entirely, rather than against the raw text.

Usage:

    contrib/drive_tui.py session.txt -- target/debug/bravebot
    contrib/drive_tui.py - --squash -- target/debug/bravebot --resume  < session.txt

A script is one step per line, `timeout` then the keys to send:

    # Answer the trust prompt, ask something, then summarise the conversation.
    5    y
    60   what is 2 plus 2\r
    60   /compact\r
    5    /exit\r

Blank lines and `#` comments are ignored. Backslash escapes are the usual ones, plus `\\e` for
escape and `\\x03` for ctrl-c.
"""

import argparse
import fcntl
import os
import pty
import re
import select
import struct
import sys
import termios
import time

#: Escape sequences, which are stripped rather than interpreted. See the note above.
ANSI = re.compile(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b[()][A-Z0-9]|\x1b[=>]|\x1b\][^\x07]*\x07")

#: What the interface draws its frames out of, which is noise to an assertion.
FURNITURE = re.compile(r"[\s─-╿⎿⎜•·]+")


def plain(captured):
    """The characters in the order they were written, with the escape sequences gone."""
    return ANSI.sub("", captured).replace("\r", "")


def squash(captured):
    """The same, with the layout removed too.

    Wrapping puts a line break and a box border through the middle of any sentence long enough to
    matter, so a search for the sentence fails on text that is plainly there. Removing every space
    and every border leaves something a substring check can be trusted against.
    """
    return FURNITURE.sub("", plain(captured))


def parse_script(text):
    """Steps as (timeout, keys), from the line format described above."""
    steps = []
    for number, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        head, _, keys = stripped.partition(" ")
        try:
            timeout = float(head)
        except ValueError:
            raise SystemExit(f"line {number}: expected a timeout in seconds, got {head!r}")
        # Decoded here rather than by the shell, so a script file can carry a carriage return
        # without depending on how it was quoted on the way in.
        steps.append((timeout, keys.strip().encode().decode("unicode_escape")))
    return steps


def drive(argv, steps, env=None, cols=120, rows=40, quiet=1.5, settle=5.0):
    """Run `argv` under a pty, send each step, and return everything it wrote."""
    child, terminal = pty.fork()
    if child == 0:
        os.environ.update(env or {})
        os.environ["TERM"] = "xterm-256color"
        os.execvp(argv[0], argv)

    # Set before the first draw, since the interface measures the terminal once at startup and
    # lays every frame out against what it found.
    fcntl.ioctl(terminal, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    captured = []

    def pump(timeout):
        """Read until the stream has been quiet for `quiet`, or `timeout` passes. False if gone."""
        deadline = time.time() + timeout
        last = time.time()
        while time.time() < deadline:
            ready, _, _ = select.select([terminal], [], [], 0.2)
            if ready:
                try:
                    chunk = os.read(terminal, 65536)
                except OSError:
                    return False
                if not chunk:
                    return False
                captured.append(chunk.decode("utf-8", "replace"))
                last = time.time()
            elif time.time() - last >= quiet:
                return True
        return True

    pump(settle)
    for timeout, keys in steps:
        if keys:
            try:
                os.write(terminal, keys.encode())
            except OSError:
                # The child is gone. Whatever it managed to say before it went is the reason, and
                # it is already captured, so stop rather than raising over the top of it.
                break
        if not pump(timeout):
            break
    pump(quiet * 2)

    try:
        os.close(terminal)
    except OSError:
        pass
    try:
        os.waitpid(child, os.WNOHANG)
    except ChildProcessError:
        pass
    return "".join(captured)


def main():
    parser = argparse.ArgumentParser(
        description="Drive the bravebot TUI from a script.",
        epilog="The command to run follows a bare --, as in: drive_tui.py s.txt -- target/debug/bravebot",
    )
    parser.add_argument("script", help="script file, or - to read the script from stdin")
    parser.add_argument("--raw", metavar="FILE", help="also write the untouched capture here")
    parser.add_argument("--squash", action="store_true",
                        help="print with layout removed, which is what to assert against")
    parser.add_argument("--cols", type=int, default=120)
    parser.add_argument("--rows", type=int, default=40)
    parser.add_argument("--quiet", type=float, default=1.5,
                        help="seconds of silence that end a step early (default: 1.5)")
    parser.add_argument("--settle", type=float, default=5.0,
                        help="seconds to wait for the first screen (default: 5)")
    # Split on the separator before argparse sees it. REMAINDER would do this only when the
    # separator comes before every option, so `--squash -- bravebot` silently ran a program called
    # --squash. Splitting here means the options may go in any order.
    argv = sys.argv[1:]
    if "--" in argv:
        cut = argv.index("--")
        mine, command = argv[:cut], argv[cut + 1 :]
    else:
        mine, command = argv, []

    # Parsed before the command is insisted on, so `--help` prints help rather than complaining
    # about a separator the reader is asking where to put.
    args = parser.parse_args(mine)
    if not command:
        parser.error("give the command to run after a --")

    text = sys.stdin.read() if args.script == "-" else open(args.script).read()
    captured = drive(command, parse_script(text), cols=args.cols, rows=args.rows,
                     quiet=args.quiet, settle=args.settle)

    if args.raw:
        with open(args.raw, "w") as handle:
            handle.write(captured)
    print(squash(captured) if args.squash else plain(captured))


if __name__ == "__main__":
    main()
