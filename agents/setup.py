#!/usr/bin/env python3
"""Link the checked-in agent configuration into each tool's discovery directory.

The source of truth is `agents/`: versioned, reviewed, and named for no particular
vendor. No tool reads it. Claude Code discovers skills, slash commands and subagents
under `.claude/`, and bravebot discovers skills under `.bravebot/skills` and its
instructions from `AGENTS.md` at the workspace root. This script bridges the two by
creating one symlink per entry, so a skill is written once and both tools see it:

    .claude/skills/<name>    ->  agents/skills/<name>
    .bravebot/skills/<name>  ->  agents/skills/<name>
    .claude/CLAUDE.md        ->  agents/AGENTS.md
    AGENTS.md                ->  agents/AGENTS.md

The generated links are gitignored and never committed, which is why this runs from
`make init` rather than being a one-time setup somebody has to remember.

It is idempotent: re-running only creates links that are missing or stale, and it never
clobbers a real file or directory somebody placed in a discovery dir by hand.

It does not touch the trust map. bravebot loads a workspace skill only from a path a
person vouched for (TRUST-1 in docs/specs/trust-map.md), and a script granting that on
the user's behalf is exactly the inference that clause forbids. Expect to be asked once,
at startup, about `.bravebot/skills`.

On Windows we COPY instead of symlink, because git's symlink support there is unreliable
and creating symlinks often needs elevated privileges.

Usage:
    python3 agents/setup.py link    # create the links (default)
    python3 agents/setup.py list    # show source vs. linked state
    python3 agents/setup.py unlink  # remove only the links we own
"""

import argparse
import logging
import os
import shutil
import sys
from pathlib import Path

# .../<repo>/agents/setup.py -> parents[1] == <repo>
_ROOT = Path(__file__).resolve().parents[1]
_SRC = _ROOT / 'agents'
_IS_WINDOWS = os.name == 'nt'

# One source directory fans out to one link per child, rather than linking the directory
# itself, so a discovery dir can also hold entries this repo does not own.
#
# bravebot reads skills only (crates/agent/src/skills.rs). Slash commands and subagents
# are Claude Code concepts, so a `commands/` or `agents/` directory here has nowhere to
# go for bravebot and is simply not linked for it.
_FANOUT = [
    ('skills', ['.claude/skills', '.bravebot/skills']),
    ('commands', ['.claude/commands']),
    ('agents', ['.claude/agents']),
]

# The one instructions file, under the name each tool looks for. bravebot reads it from
# the workspace root (crates/agent/src/preamble.rs); Claude Code reads `.claude/CLAUDE.md`.
_FILES = [
    ('AGENTS.md', ['.claude/CLAUDE.md', 'AGENTS.md']),
]

# A child of a fanned-out source dir is only worth linking if it is a real entry rather
# than a stray file, so skills are recognised the way both tools recognise them.
_MARKERS = {'skills': ('SKILL.md', 'skill.md')}


def _log(msg, *args):
    logging.info(msg, *args)


def _is_entry(path: Path, kind: str) -> bool:
    markers = _MARKERS.get(kind)
    if markers is None:
        return path.is_dir() or path.suffix == '.md'
    return any((path / marker).exists() for marker in markers)


def planned_links() -> list[tuple[Path, Path]]:
    """Every (source, destination) pair this script owns, in link order.

    Sources that do not exist yet are skipped rather than reported, so adding a
    `commands/` directory later needs no change here.
    """
    links: list[tuple[Path, Path]] = []

    for kind, dests in _FANOUT:
        src_dir = _SRC / kind
        if not src_dir.is_dir():
            continue
        for entry in sorted(src_dir.iterdir()):
            if entry.name.startswith('.') or not _is_entry(entry, kind):
                continue
            for dest in dests:
                links.append((entry, _ROOT / dest / entry.name))

    for name, dests in _FILES:
        src = _SRC / name
        if not src.is_file():
            continue
        for dest in dests:
            links.append((src, _ROOT / dest))

    return links


def _link_one(src: Path, dest: Path) -> tuple[bool, bool]:
    """Create or refresh one link (or copy on Windows).

    Returns (ok, changed). `changed` is False when the link was already correct or a real
    path was left alone, so a re-run with nothing to do stays silent.
    """
    rel = dest.relative_to(_ROOT)

    if dest.is_symlink():
        try:
            if dest.resolve() == src.resolve():
                return True, False
        except OSError:
            pass  # broken link, replaced below
        _log('  refreshing stale link: %s', rel)
        dest.unlink()
    elif dest.exists():
        if _IS_WINDOWS:
            # Our own copy from a previous run. Refresh it.
            shutil.rmtree(dest) if dest.is_dir() else dest.unlink()
        else:
            # Somebody's real file or directory. Warning level so it survives -q.
            logging.warning('  SKIP %s: a real path is already there (left as-is)', rel)
            return True, False

    dest.parent.mkdir(parents=True, exist_ok=True)
    if _IS_WINDOWS:
        shutil.copytree(src, dest) if src.is_dir() else shutil.copy2(src, dest)
        logging.debug('  copied  %s', rel)
    else:
        # A relative target keeps the link valid if the checkout moves.
        os.symlink(os.path.relpath(src, dest.parent), dest,
                   target_is_directory=src.is_dir())
        logging.debug('  linked  %s', rel)
    return True, True


def link(_args) -> bool:
    links = planned_links()
    if not links:
        _log('Nothing to link: %s holds no skills, commands or AGENTS.md.', _SRC)
        return True

    ok = True
    changed = 0
    for src, dest in links:
        try:
            one_ok, one_changed = _link_one(src, dest)
            ok = one_ok and ok
            changed += one_changed
        except OSError as e:
            logging.error('  ERROR linking %s: %s', dest.relative_to(_ROOT), e)
            ok = False

    if changed:
        _log('Linked %d path(s) from agents/ into .claude/ and .bravebot/', changed)
    return ok


def unlink(_args) -> bool:
    """Remove only links pointing into agents/, never a real path."""
    for src, dest in planned_links():
        if dest.is_symlink():
            try:
                if dest.resolve() != src.resolve():
                    continue
            except OSError:
                pass  # broken link into our own tree, safe to drop
            dest.unlink()
            _log('  unlinked %s', dest.relative_to(_ROOT))
        elif dest.exists() and _IS_WINDOWS:
            shutil.rmtree(dest) if dest.is_dir() else dest.unlink()
            _log('  removed copy %s', dest.relative_to(_ROOT))
    return True


def show(_args) -> bool:
    links = planned_links()
    print(f'source: {_SRC}\n')
    if not links:
        print('(nothing to link)')
        return True

    width = max(len(str(d.relative_to(_ROOT))) for _, d in links)
    print(f'{"DESTINATION".ljust(width)}  STATE')
    print(f'{"-" * width}  -----')
    for src, dest in links:
        if dest.is_symlink():
            state = 'link' if dest.exists() else 'BROKEN link'
        elif dest.exists():
            state = 'real path (not ours)'
        else:
            state = 'missing'
        print(f'{str(dest.relative_to(_ROOT)).ljust(width)}  {state}')
    return True


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command',
                        choices=['link', 'unlink', 'list'],
                        nargs='?',
                        default='link')
    parser.add_argument('-q',
                        '--quiet',
                        action='store_true',
                        help='Only log warnings and errors.')
    args = parser.parse_args()

    logging.basicConfig(level=logging.WARNING if args.quiet else logging.INFO,
                        format='%(message)s')

    handler = {'link': link, 'unlink': unlink, 'list': show}[args.command]
    return 0 if handler(args) else 1


if __name__ == '__main__':
    sys.exit(main())
