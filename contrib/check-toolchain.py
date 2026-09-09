#!/usr/bin/env python3
"""Says when the host toolchain is old enough that clippy here is weaker than the one CI runs.

CI installs whatever stable is on the day it runs, and clippy gains lints with every release, so a
host a few releases behind passes a lint that CI fails. Nothing in a diff shows this, and the
report arrives as a CI failure on code that was checked before it was pushed.

Rust ships every six weeks and rustc states its own release date, so the gap is measurable without
asking the network what stable is today.
"""

import os
import re
# Only ever invoked with an argument list, never a shell string.
import subprocess  # nosemgrep: gitlab.bandit.B404
import sys
from datetime import date, datetime

# Six weeks, plus a few days, because a release occasionally slips and a false alarm on the day of
# one costs more than noticing a stale toolchain three days late.
CYCLE_DAYS = 42
SLACK_DAYS = 3

BANNER = re.compile(r"^rustc (\d+\.\d+\.\d+)[^(]*\(\w+ (\d{4}-\d{2}-\d{2})\)")


def released():
    """The version the host runs and the day it came out, or None when rustc does not say."""
    try:
        banner = subprocess.run(
            ["rustc", "--version"], capture_output=True, text=True, timeout=30
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    found = BANNER.match(banner.strip())
    if not found:
        return None
    return found.group(1), datetime.strptime(found.group(2), "%Y-%m-%d").date()


def main():
    # The same shape as BRAVEBOT_ALLOW_UNCONFIGURED_BUILD: a way to work on a host that cannot have
    # the newer toolchain, set deliberately rather than by editing the check out.
    if os.environ.get("BRAVEBOT_ALLOW_STALE_TOOLCHAIN"):
        return 0

    # An unreadable banner is not evidence of anything, and a check that guesses in the absence of
    # evidence is a check people learn to ignore.
    found = released()
    if found is None:
        return 0
    version, released_on = found

    days = (date.today() - released_on).days
    if days < CYCLE_DAYS + SLACK_DAYS:
        return 0

    behind = days // CYCLE_DAYS
    warn = "--warn" in sys.argv[1:]
    print(
        f"\nrustc {version} came out on {released_on}, {days} days ago, so stable is about "
        f"{behind} release{'' if behind == 1 else 's'} ahead of it.\n"
        "CI lints with current stable, and clippy gains lints between releases, so clippy here\n"
        "passes warnings CI fails on.\n\n"
        "  make check-linux                     the fmt, clippy and tests CI runs, on stable\n"
        "  rustup update stable                 or, on a homebrew rust, brew upgrade rust\n"
        "  BRAVEBOT_ALLOW_STALE_TOOLCHAIN=1     to say this is known and proceed anyway",
        file=sys.stderr,
    )
    return 0 if warn else 1


if __name__ == "__main__":
    sys.exit(main())
