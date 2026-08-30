#!/usr/bin/env python3
"""Check that the implementation matches docs/specs.

Two passes. This script is the first: everything that can be decided without a model,
which is most of the bookkeeping a spec carries. Clause numbering, the tests a clause
names, the paths it governs, the symbols it guards, and the table in the specs README
are all facts, and a fact does not need a review.

    check-spec.py --mechanical-only          the whole first pass, human readable
    check-spec.py --mechanical-only labels   one spec, by name or by id
    check-spec.py --changed                  only specs governing what this branch touched

Run with neither --mechanical-only nor --list it also prepares the second pass: a work
directory holding one prompt file per group of clauses, for the conformance review the
check-spec skill drives. Nothing about that pass is decided here; this script writes
files and prints a pointer to them.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from fnmatch import fnmatch
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from specs import EM_DASH, README, SPEC_DIR, TestIndex, crate_directories, load_specs  # noqa: E402

ERROR = "error"
WARNING = "warning"


def finding(spec, severity, kind, summary, clause=None, evidence=None, fix=None):
    return {
        "spec": spec,
        "clause": clause,
        "severity": severity,
        "kind": kind,
        "summary": summary,
        "evidence": evidence,
        "fix": fix,
        "source": "mechanical",
    }


def check_front_matter(spec):
    if spec.malformed_front_matter:
        yield finding(
            spec.rel,
            ERROR,
            "front-matter-malformed",
            "front matter is not the scalar-and-list shape the specs README describes",
        )
        return
    for required in ("id", "title", "status"):
        if not spec.front.get(required):
            yield finding(
                spec.rel, ERROR, "front-matter-missing", f"front matter has no `{required}`"
            )
    if not spec.governs:
        yield finding(
            spec.rel,
            ERROR,
            "front-matter-missing",
            "front matter lists no `governs` paths, so no diff is ever reviewed against it",
        )


def check_clause_numbering(spec):
    seen = {}
    expected = 1
    for clause in spec.clauses:
        if clause.prefix != spec.id:
            yield finding(
                spec.rel,
                ERROR,
                "clause-prefix-mismatch",
                f"clause is `{clause.id}` in a spec whose id is `{spec.id}`",
                clause=clause.id,
                evidence=f"{spec.rel}:{clause.line}",
            )
        if clause.number in seen:
            yield finding(
                spec.rel,
                ERROR,
                "clause-duplicate",
                f"`{clause.id}` appears twice, and an id points at one clause forever",
                clause=clause.id,
                evidence=f"{spec.rel}:{clause.line} and {spec.rel}:{seen[clause.number]}",
            )
        seen[clause.number] = clause.line
        if clause.number != expected:
            yield finding(
                spec.rel,
                ERROR,
                "clause-numbering",
                f"`{clause.id}` follows {expected - 1}: ids are allocated in order and never renumbered",
                clause=clause.id,
                evidence=f"{spec.rel}:{clause.line}",
                fix="a withdrawn clause stays in place, marked withdrawn, rather than leaving a gap",
            )
        expected = max(expected, clause.number) + 1
        if clause.withdrawn and "replaced" not in clause.text.lower():
            yield finding(
                spec.rel,
                ERROR,
                "withdrawn-without-replacement",
                f"`{clause.id}` is withdrawn but does not say what replaced it",
                clause=clause.id,
                evidence=f"{spec.rel}:{clause.line}",
            )


def check_coverage(spec, index, crates):
    for clause in spec.clauses:
        if clause.withdrawn:
            continue
        if not clause.verified_by:
            yield finding(
                spec.rel,
                ERROR,
                "clause-unverified",
                f"`{clause.id}` names no test at all: every clause carries a `verified-by` line",
                clause=clause.id,
                evidence=f"{spec.rel}:{clause.line}",
            )
            continue
        for reference in clause.verified_by:
            if reference == "none":
                yield finding(
                    spec.rel,
                    WARNING,
                    "clause-uncovered",
                    f"`{clause.id}` is `verified-by: none`, so nothing pins it",
                    clause=clause.id,
                    evidence=f"{spec.rel}:{clause.line}",
                    fix="write the test, or say by-construction with the reason in brackets",
                )
                continue
            if reference.startswith("by-construction"):
                rest = reference[len("by-construction") :].strip()
                bracketed = (rest[:1], rest[-1:]) in (("(", ")"), ("[", "]"))
                if not (bracketed and len(rest) > 2):
                    yield finding(
                        spec.rel,
                        ERROR,
                        "by-construction-unexplained",
                        f"`{clause.id}` says by-construction without saying in brackets what makes it hold",
                        clause=clause.id,
                        evidence=f"{spec.rel}:{clause.line}",
                    )
                continue
            status, detail = index.resolve(reference, crates)
            if status == "ok":
                continue
            if status == "moved":
                yield finding(
                    spec.rel,
                    ERROR,
                    "verified-by-moved",
                    f"`{clause.id}` names `{reference}`, but that test lives elsewhere now",
                    clause=clause.id,
                    evidence=detail,
                    fix="update the path in the spec, or move the test back to the module it belongs to",
                )
            elif status == "missing":
                yield finding(
                    spec.rel,
                    ERROR,
                    "verified-by-missing",
                    f"`{clause.id}` names `{reference}`, and no `#[test]` by that name exists",
                    clause=clause.id,
                    evidence=f"{spec.rel}:{clause.line}",
                )
            elif status == "unknown-crate":
                yield finding(
                    spec.rel,
                    ERROR,
                    "verified-by-unknown-crate",
                    f"`{clause.id}` names crate `{detail}`, which is not in this workspace",
                    clause=clause.id,
                    evidence=f"{spec.rel}:{clause.line}",
                )
            else:
                yield finding(
                    spec.rel,
                    ERROR,
                    "verified-by-malformed",
                    f"`{clause.id}` names `{reference}`, which is not `crate::module::test_name`",
                    clause=clause.id,
                    evidence=f"{spec.rel}:{clause.line}",
                )


def check_governs(spec):
    for pattern in spec.governs:
        if any(character in pattern for character in "*?["):
            if not list(Path().glob(pattern)):
                yield finding(
                    spec.rel,
                    ERROR,
                    "governs-missing",
                    f"`governs` pattern `{pattern}` matches nothing",
                )
        elif not Path(pattern).exists():
            yield finding(
                spec.rel,
                ERROR,
                "governs-missing",
                f"`governs` names `{pattern}`, which does not exist",
                fix="a rename that leaves governs behind quietly stops the spec reviewing anything",
            )


def guard_sites(symbol, sources):
    """Where a guarded symbol is named. `Type::method` also matches `.method(`, since
    that is how most call sites read once the receiver has a type.

    The definition is matched on the whole name rather than on a prefix. A guard named
    `vouch` matching `fn vouching_for_one_command` would report a renamed symbol as
    present, which is the one answer this check must never give."""
    bare = symbol.split("::")[-1]
    definition = re.compile(rf"\bfn\s+{re.escape(bare)}\s*[(<]")
    call = f".{bare}("
    hits = []
    for path, lines in sources.items():
        for number, raw in enumerate(lines, start=1):
            if symbol in raw or call in raw or definition.search(raw):
                hits.append((str(path), number, raw.strip()))
    return hits


def check_guards(spec, sources):
    for symbol in spec.guards:
        if not guard_sites(symbol, sources):
            yield finding(
                spec.rel,
                ERROR,
                "guard-missing",
                f"`guards` names `{symbol}`, which appears nowhere in crates/",
                fix="a guarded symbol that was renamed stops being review-required silently",
            )


def check_isolation(spec, prefixes):
    """Each spec stands alone: a clause id is only ever cited inside its own file."""
    for other in prefixes:
        if other == spec.id:
            continue
        for number, raw in enumerate(spec.lines, start=1):
            for token in ("`" + other + "-", " " + other + "-", "(" + other + "-"):
                if token in raw:
                    yield finding(
                        spec.rel,
                        ERROR,
                        "cross-spec-citation",
                        f"cites `{other}`'s clause ids; a reader landing here has no idea what they are",
                        evidence=f"{spec.rel}:{number}",
                        fix="state the fact in a few words, and link the other file by name",
                    )
                    break
            else:
                continue
            break


def check_prose(spec):
    for number, raw in enumerate(spec.lines, start=1):
        if EM_DASH in raw:
            yield finding(
                spec.rel,
                ERROR,
                "em-dash",
                "em-dash: a comma, a colon, parentheses, or two sentences will do the job",
                evidence=f"{spec.rel}:{number}",
            )


def check_readme(specs):
    if not README.exists():
        yield finding(str(README), ERROR, "readme-missing", "docs/specs/README.md is not there")
        return
    text = README.read_text(encoding="utf-8")
    rows = {}
    for raw in text.split("\n"):
        if not raw.startswith("|"):
            continue
        cells = [cell.strip() for cell in raw.strip("|").split("|")]
        if len(cells) < 4 or not cells[0].startswith("["):
            continue
        link = cells[0].split("(", 1)[1].rstrip(")") if "(" in cells[0] else cells[0]
        rows[link] = (cells[1].strip("`"), cells[2])

    for spec in specs:
        link = str(spec.path.relative_to(SPEC_DIR))
        if link not in rows:
            yield finding(
                str(README),
                ERROR,
                "readme-unlisted",
                f"{link} is a spec but has no row in the table",
                fix="an unlisted spec is one nobody finds, and its topic reads as ordinary code",
            )
            continue
        listed_id, listed_count = rows.pop(link)
        if listed_id != spec.id:
            yield finding(
                str(README),
                ERROR,
                "readme-wrong-id",
                f"{link} is listed as `{listed_id}` and its front matter says `{spec.id}`",
            )
        live = len([c for c in spec.clauses if not c.withdrawn])
        if listed_count.isdigit() and int(listed_count) != live:
            yield finding(
                str(README),
                ERROR,
                "readme-wrong-count",
                f"{link} is listed with {listed_count} clauses and has {live}",
            )

    for link in rows:
        yield finding(
            str(README),
            ERROR,
            "readme-phantom",
            f"the table lists {link}, which is not a spec file",
        )


def load_sources():
    return {
        path: path.read_text(encoding="utf-8", errors="replace").split("\n")
        for path in sorted(Path("crates").rglob("*.rs"))
    }


def changed_files(base):
    for command in (
        ["git", "diff", "--name-only", f"{base}...HEAD"],
        ["git", "diff", "--name-only", "HEAD"],
        ["git", "diff", "--name-only", "--cached"],
    ):
        try:
            out = subprocess.run(command, capture_output=True, text=True, check=False).stdout
        except OSError:
            continue
        for line in out.split("\n"):
            if line.strip():
                yield line.strip()


def select(specs, selectors, changed_base):
    if selectors:
        wanted = {s.lower().removesuffix(".md") for s in selectors}
        chosen = [
            spec
            for spec in specs
            if spec.path.stem.lower() in wanted or spec.id.lower() in wanted or spec.name.lower() in wanted
        ]
        missing = wanted - {spec.path.stem.lower() for spec in chosen} - {spec.id.lower() for spec in chosen}
        return chosen, sorted(missing)
    if changed_base is not None:
        touched = set(changed_files(changed_base))
        chosen = []
        for spec in specs:
            if spec.rel in touched or any(
                any(fnmatch(f, pattern) for f in touched) for pattern in spec.governs
            ):
                chosen.append(spec)
        return chosen, []
    return list(specs), []


PROMPT = """\
# Conformance review: {spec_name}, clauses {clause_range}

You are checking that the **implementation matches the spec**, not the other way round.

## The one direction this review runs in

When the code and the spec disagree, **the code is wrong**. Never propose editing a clause
to describe what the code does, never soften a clause, and never report "the spec is
unrealistic" as a finding. If an implementation cannot satisfy a clause, the implementation
is the finding. Proposing spec text is out of scope for this review entirely: another
process, with human review, changes specs.

Clauses describe behaviour, not implementation. A clause is satisfied by behaviour that
holds, not by a function that happens to be named after it.

## The rule that outranks every clause

The driver and the planner never have untrusted content in their context. The driver is
`bravebot-core` and `bravebot-agent` both; the planner is the model. The driver may carry
untrusted content and hand it to an effect, but may not branch on it: no `if`, `match`,
comparison, or early return whose condition derives from untrusted bytes. Moving such a
branch from `bravebot-agent` into `bravebot-core` does not fix it.

A witness is not permission to inspect. A `declassify` outside `Policy::present`,
`Policy::render_in_place` and `Policy::read_trusted_content` is almost certainly a
violation. Constructing a `Labelled` by hand to give a value a better label than its
inputs had is laundering.

Two deliberate exceptions are listed under "Known costs" in docs/specs/labels.md. Anything
not listed there is a violation, whatever the comment beside it says.

Report a breach of this rule at severity `error`, whether or not a clause names it.

## What you are reviewing

Spec file: `{spec_path}`
Clauses in your scope: {clause_ids}

Read these yourself, with your own tools. They are named rather than pasted so you can read
what the clauses actually need.

Governed source files (this spec decides these paths):
{governed}

Guarded symbols and every place they are named (each use is review-required):
{guards}

Tests the clauses in your scope name, already confirmed to exist:
{tests}

## The spec, in full

Read every clause for context. Report only on the ones in your scope.

```markdown
{spec_text}
```

## The job

For each clause in your scope:

1. Read the governed source until you can say what the code actually does, in the terms
   the clause uses. Read the named tests too.
2. Decide one of:
   - `conforms`: the behaviour the clause describes is the behaviour the code has.
   - `violation`: the code does something the clause forbids, or does not do something the
     clause requires. This is a finding.
   - `untested`: the behaviour looks right, but the named tests do not actually pin it. A
     test that would still pass against the buggy code does not pin anything. This is a
     finding at severity `warning`, and the fix is a test, not a spec edit.
   - `unclear`: you could not tell from the governed files. Say what you would need. Not a
     finding, but say so honestly rather than guessing `conforms`.
3. Every `violation` needs a concrete failure: the input or state, the path through the
   code, and the outcome the clause forbids. `file:line` for each step. A finding you
   cannot walk somebody through is a finding you have not verified, and it does not go in.

Check the whole clause, including the parts stated in the "Why" paragraph where the clause
has one, and any table a clause carries: a table row is part of that clause.

Before you report a violation, look for the code that would refute it. Structural
guarantees in this repository are often enforced somewhere other than the place you are
reading, in a type that offers no way to do the wrong thing. Take the refutation seriously.
False findings on a spec check are expensive: they get somebody to weaken a guarantee that
was already holding.

## Output

Write JSON to `{results_file}` and nothing else to stdout:

```json
{{
  "spec": "{spec_name}",
  "clauses": [
    {{
      "clause": "{example_clause}",
      "verdict": "conforms | violation | untested | unclear",
      "summary": "one sentence, only when the verdict is not conforms",
      "severity": "error | warning",
      "evidence": ["crates/.../file.rs:123 what is there"],
      "failure": "input or state, path through the code, outcome the clause forbids",
      "fix": "what the implementation should do instead, in a sentence"
    }}
  ]
}}
```

One entry per clause in your scope, including the ones that conform. No prose outside the
file. Do not edit any file.
"""


def build_prompt(spec, clauses, sources, crates, index, results_file):
    governed = "\n".join(
        f"- `{pattern}`" + ("" if Path(pattern).exists() else "  (missing)")
        for pattern in spec.governs
    ) or "- (none)"

    guard_lines = []
    for symbol in spec.guards:
        sites = guard_sites(symbol, sources)
        guard_lines.append(f"- `{symbol}`: {len(sites)} sites")
        for path, number, _ in sites[:40]:
            guard_lines.append(f"    - {path}:{number}")
        if len(sites) > 40:
            guard_lines.append(f"    - ... and {len(sites) - 40} more, grep for it")
    guards = "\n".join(guard_lines) or "- (none)"

    test_lines = []
    for clause in clauses:
        for reference in clause.verified_by:
            if reference == "none" or reference.startswith("by-construction"):
                test_lines.append(f"- {clause.id}: `{reference}` (nothing to read)")
                continue
            status, detail = index.resolve(reference, crates)
            location = detail if status in ("ok", "moved") else "unresolved"
            test_lines.append(f"- {clause.id}: `{reference}` at {location}")
    tests = "\n".join(test_lines) or "- (none)"

    ids = [clause.id for clause in clauses]
    return PROMPT.format(
        spec_name=spec.name,
        spec_path=spec.rel,
        clause_range=f"{ids[0]} to {ids[-1]}" if len(ids) > 1 else ids[0],
        clause_ids=", ".join(f"`{i}`" for i in ids),
        governed=governed,
        guards=guards,
        tests=tests,
        spec_text=spec.path.read_text(encoding="utf-8"),
        results_file=results_file,
        example_clause=ids[0],
    )


def render(findings, chosen, strict):
    errors = [f for f in findings if f["severity"] == ERROR]
    warnings = [f for f in findings if f["severity"] == WARNING]
    lines = []
    by_spec = {}
    for item in findings:
        by_spec.setdefault(item["spec"], []).append(item)
    for spec in sorted(by_spec):
        lines.append(f"\n{spec}")
        for item in by_spec[spec]:
            mark = "error" if item["severity"] == ERROR else "warn "
            where = f"  [{item['evidence']}]" if item.get("evidence") else ""
            lines.append(f"  {mark}  {item['summary']}{where}")
            if item.get("fix"):
                lines.append(f"         {item['fix']}")
    clauses = sum(len(spec.clauses) for spec in chosen)
    lines.append("")
    lines.append(
        f"{len(chosen)} specs, {clauses} clauses: {len(errors)} errors, {len(warnings)} warnings"
    )
    if warnings and not strict:
        lines.append("warnings do not fail this check; --strict makes them fail")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("specs", nargs="*", help="spec file names, stems, or ids; default all")
    parser.add_argument("--mechanical-only", action="store_true", help="skip the review pass")
    parser.add_argument("--changed", nargs="?", const="main", default=None, metavar="BASE")
    parser.add_argument("--strict", action="store_true", help="warnings fail too")
    parser.add_argument("--work-dir", default=None)
    parser.add_argument("--chunk-size", type=int, default=6, help="clauses per reviewer")
    parser.add_argument("--json", action="store_true", help="findings as JSON on stdout")
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[3]
    os.chdir(root)
    if not SPEC_DIR.is_dir():
        print(f"no {SPEC_DIR} under {root}", file=sys.stderr)
        return 2

    specs = load_specs()
    chosen, unknown = select(specs, args.specs, args.changed)
    findings = [
        finding(str(SPEC_DIR), ERROR, "unknown-spec", f"no spec named `{name}`")
        for name in unknown
    ]

    sources = load_sources()
    crates = crate_directories()
    index = TestIndex()
    prefixes = {spec.id for spec in specs if spec.id}

    for spec in chosen:
        findings.extend(check_front_matter(spec))
        findings.extend(check_clause_numbering(spec))
        findings.extend(check_coverage(spec, index, crates))
        findings.extend(check_governs(spec))
        findings.extend(check_guards(spec, sources))
        findings.extend(check_isolation(spec, prefixes))
        findings.extend(check_prose(spec))
    if len(chosen) == len(specs):
        findings.extend(check_readme(specs))

    failed = any(f["severity"] == ERROR for f in findings) or (
        args.strict and bool(findings)
    )

    if args.mechanical_only or args.json:
        if args.json:
            print(json.dumps({"findings": findings}, indent=2))
        else:
            print(render(findings, chosen, args.strict))
        return 1 if failed else 0

    work_dir = Path(args.work_dir or tempfile.mkdtemp(prefix="check-spec-"))
    work_dir.mkdir(parents=True, exist_ok=True)
    entries = []
    for spec in chosen:
        reviewable = [c for c in spec.clauses if not c.withdrawn]
        if not reviewable:
            continue
        spec_dir = work_dir / (spec.id or spec.path.stem)
        spec_dir.mkdir(parents=True, exist_ok=True)
        chunks = [
            reviewable[i : i + args.chunk_size]
            for i in range(0, len(reviewable), args.chunk_size)
        ]
        prompts = []
        for number, clauses in enumerate(chunks, start=1):
            results_file = spec_dir / f"chunk{number}_results.json"
            prompt_file = spec_dir / f"chunk{number}_prompt.md"
            prompt_file.write_text(
                build_prompt(spec, clauses, sources, crates, index, results_file), encoding="utf-8"
            )
            prompts.append(
                {
                    "prompt_file": str(prompt_file),
                    "results_file": str(results_file),
                    "clauses": [c.id for c in clauses],
                }
            )
        entries.append(
            {
                "spec": spec.name,
                "path": spec.rel,
                "id": spec.id,
                "clauses": len(reviewable),
                "reviewer_prompts": prompts,
            }
        )

    manifest = {
        "generated": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "root": str(root),
        "strict": args.strict,
        "mechanical_findings": findings,
        "mechanical_failed": failed,
        "specs": entries,
        "progress_lines": [
            f"check-spec: {len(entries)} specs, "
            f"{sum(len(e['reviewer_prompts']) for e in entries)} reviewers, "
            f"{len(findings)} mechanical findings"
        ],
    }
    manifest_path = work_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    print(render(findings, chosen, args.strict), file=sys.stderr)
    print(json.dumps({"work_dir": str(work_dir), "manifest": str(manifest_path)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
