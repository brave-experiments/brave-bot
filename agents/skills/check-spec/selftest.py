#!/usr/bin/env python3
"""Prove each mechanical check fires.

A check that never fires is worse than no check: it reports a clean spec tree forever and
somebody trusts it. Each case here builds a small fixture repository, breaks exactly one
thing, and asserts that exactly that check reports it. The first case breaks nothing, so a
check that fires on anything at all is caught too.

    python3 agents/skills/check-spec/selftest.py
"""

import os
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import importlib.util  # noqa: E402

from specs import TestIndex, crate_directories, load_specs  # noqa: E402

spec = importlib.util.spec_from_file_location(
    "check_spec", Path(__file__).resolve().parent / "check-spec.py"
)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


CLEAN_SPEC = """\
---
id: DEMO
title: A demonstration
status: normative
governs:
  - crates/demo/src/lib.rs
guards:
  - symbol: Gate::open
---

## Clauses

### DEMO-1: the gate opens only once

`verified-by: bravebot_demo::lib::the_gate_opens_only_once`

### DEMO-2: nothing else can open it

`verified-by: by-construction (the field is private)`
"""

CLEAN_README = """\
# Specs

| Spec | Id | Clauses | Topic |
|---|---|---|---|
| [demo.md](demo.md) | `DEMO` | 2 | a demonstration |
"""

CLEAN_SOURCE = """\
pub struct Gate;

impl Gate {
    pub fn open(&self) {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_gate_opens_only_once() {}
}
"""


def build_fixture(root):
    (root / "docs" / "specs").mkdir(parents=True)
    (root / "docs" / "specs" / "demo.md").write_text(CLEAN_SPEC, encoding="utf-8")
    (root / "docs" / "specs" / "README.md").write_text(CLEAN_README, encoding="utf-8")
    (root / "crates" / "demo" / "src").mkdir(parents=True)
    (root / "crates" / "demo" / "Cargo.toml").write_text(
        '[package]\nname = "bravebot-demo"\n', encoding="utf-8"
    )
    (root / "crates" / "demo" / "src" / "lib.rs").write_text(CLEAN_SOURCE, encoding="utf-8")


def run_checks():
    specs = load_specs()
    sources = check.load_sources()
    crates = crate_directories()
    index = TestIndex()
    prefixes = {s.id for s in specs if s.id}
    findings = []
    for one in specs:
        findings.extend(check.check_front_matter(one))
        findings.extend(check.check_clause_numbering(one))
        findings.extend(check.check_coverage(one, index, crates))
        findings.extend(check.check_governs(one))
        findings.extend(check.check_guards(one, sources))
        findings.extend(check.check_isolation(one, prefixes))
        findings.extend(check.check_prose(one))
    findings.extend(check.check_readme(specs))
    return findings


def edit_spec(root, old, new):
    path = root / "docs" / "specs" / "demo.md"
    text = path.read_text(encoding="utf-8")
    assert old in text, f"fixture does not contain {old!r}"
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


def cite_another_spec(root):
    """A citation is only recognisable as one when the prefix belongs to a real spec, so
    the fixture grows a second spec for this case."""
    (root / "docs" / "specs" / "other.md").write_text(
        CLEAN_SPEC.replace("DEMO", "OTHER"), encoding="utf-8"
    )
    edit_spec(root, "the gate opens only once", "the same rule `OTHER-1` states")


CASES = [
    ("a clean spec tree reports nothing", lambda root: None, None),
    (
        "a spec with no id",
        lambda root: edit_spec(root, "id: DEMO\n", ""),
        "front-matter-missing",
    ),
    (
        "a spec governing nothing",
        lambda root: edit_spec(root, "governs:\n  - crates/demo/src/lib.rs\n", ""),
        "front-matter-missing",
    ),
    (
        "a clause numbered out of order",
        lambda root: edit_spec(root, "### DEMO-2:", "### DEMO-4:"),
        "clause-numbering",
    ),
    (
        "the same clause id twice",
        lambda root: edit_spec(root, "### DEMO-2:", "### DEMO-1:"),
        "clause-duplicate",
    ),
    (
        "a clause id from another spec's series",
        lambda root: edit_spec(root, "### DEMO-1:", "### OTHER-1:"),
        "clause-prefix-mismatch",
    ),
    (
        "a clause with no verified-by at all",
        lambda root: edit_spec(
            root, "`verified-by: bravebot_demo::lib::the_gate_opens_only_once`\n", ""
        ),
        "clause-unverified",
    ),
    (
        "a clause verified by nothing",
        lambda root: edit_spec(
            root, "bravebot_demo::lib::the_gate_opens_only_once`", "none`"
        ),
        "clause-uncovered",
    ),
    (
        "by-construction without a reason",
        lambda root: edit_spec(root, "by-construction (the field is private)", "by-construction"),
        "by-construction-unexplained",
    ),
    (
        "a verified-by naming a test that does not exist",
        lambda root: edit_spec(root, "the_gate_opens_only_once`", "the_gate_never_opens`"),
        "verified-by-missing",
    ),
    (
        "a verified-by naming a test in the wrong module",
        lambda root: edit_spec(root, "bravebot_demo::lib::", "bravebot_demo::gate::"),
        "verified-by-moved",
    ),
    (
        "a verified-by naming a crate outside the workspace",
        lambda root: edit_spec(root, "bravebot_demo::", "bravebot_ghost::"),
        "verified-by-unknown-crate",
    ),
    (
        "a governs path that was renamed away",
        lambda root: (root / "crates" / "demo" / "src" / "lib.rs").rename(
            root / "crates" / "demo" / "src" / "gate.rs"
        ),
        "governs-missing",
    ),
    (
        "a guarded symbol that was renamed away",
        lambda root: edit_spec(root, "symbol: Gate::open", "symbol: Gate::unlock"),
        "guard-missing",
    ),
    (
        "a spec citing another spec's clause ids",
        lambda root: cite_another_spec(root),
        "cross-spec-citation",
    ),
    (
        "an em-dash",
        lambda root: edit_spec(root, "the gate opens only once", "the gate opens — once"),
        "em-dash",
    ),
    (
        "a spec missing from the README table",
        lambda root: (root / "docs" / "specs" / "README.md").write_text(
            "# Specs\n", encoding="utf-8"
        ),
        "readme-unlisted",
    ),
    (
        "a README row with the wrong clause count",
        lambda root: (root / "docs" / "specs" / "README.md").write_text(
            CLEAN_README.replace("| 2 |", "| 5 |"), encoding="utf-8"
        ),
        "readme-wrong-count",
    ),
    (
        "a README row with the wrong id",
        lambda root: (root / "docs" / "specs" / "README.md").write_text(
            CLEAN_README.replace("`DEMO`", "`OTHER`"), encoding="utf-8"
        ),
        "readme-wrong-id",
    ),
    (
        "a README row for a spec that is not there",
        lambda root: (root / "docs" / "specs" / "README.md").write_text(
            CLEAN_README + "| [gone.md](gone.md) | `GONE` | 1 | nothing |\n", encoding="utf-8"
        ),
        "readme-phantom",
    ),
]


def main():
    original = Path.cwd()
    failures = []
    for name, break_it, expected in CASES:
        root = Path(tempfile.mkdtemp(prefix="check-spec-selftest-"))
        try:
            build_fixture(root)
            os.chdir(root)
            if break_it is not None:
                break_it(root)
            kinds = sorted({f["kind"] for f in run_checks()})
            if expected is None:
                ok = not kinds
                detail = f"reported {kinds}"
            else:
                ok = expected in kinds
                detail = f"reported {kinds}, wanted {expected}"
            print(f"{'ok  ' if ok else 'FAIL'}  {name}")
            if not ok:
                failures.append(f"{name}: {detail}")
        finally:
            os.chdir(original)
            shutil.rmtree(root, ignore_errors=True)

    print()
    if failures:
        for failure in failures:
            print(f"  {failure}")
        print(f"{len(failures)} of {len(CASES)} cases failed")
        return 1
    print(f"{len(CASES)} cases pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
