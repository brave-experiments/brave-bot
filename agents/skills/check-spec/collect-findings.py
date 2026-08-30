#!/usr/bin/env python3
"""Collect the conformance reviewers' results and render the check-spec report.

Reads the work directory that check-spec.py prepared, merges what the reviewers wrote
with the mechanical findings already in the manifest, and prints one report. Exits
non-zero when anything at severity error survives, so the skill and a shell both get the
same answer from the same place.
"""

import argparse
import json
import sys
from pathlib import Path

ERROR = "error"
WARNING = "warning"

VERDICT_SEVERITY = {"violation": ERROR, "untested": WARNING}


def load_reviews(manifest):
    findings = []
    reviewed = 0
    for entry in manifest["specs"]:
        for prompt in entry["reviewer_prompts"]:
            path = Path(prompt["results_file"])
            if not path.exists():
                findings.append(
                    {
                        "spec": entry["path"],
                        "clause": ", ".join(prompt["clauses"]),
                        "severity": ERROR,
                        "kind": "review-incomplete",
                        "summary": "no reviewer result: these clauses were never checked",
                        "evidence": str(path),
                        "fix": "rerun the reviewer for this chunk before trusting the report",
                        "source": "review",
                    }
                )
                continue
            try:
                result = json.loads(path.read_text(encoding="utf-8"))
            except (json.JSONDecodeError, OSError) as problem:
                findings.append(
                    {
                        "spec": entry["path"],
                        "clause": ", ".join(prompt["clauses"]),
                        "severity": ERROR,
                        "kind": "review-unreadable",
                        "summary": f"reviewer result could not be read: {problem}",
                        "evidence": str(path),
                        "source": "review",
                    }
                )
                continue

            expected = set(prompt["clauses"])
            for clause in result.get("clauses", []):
                reviewed += 1
                expected.discard(clause.get("clause"))
                verdict = clause.get("verdict", "unclear")
                if verdict == "conforms":
                    continue
                if verdict == "unclear":
                    findings.append(
                        {
                            "spec": entry["path"],
                            "clause": clause.get("clause"),
                            "severity": WARNING,
                            "kind": "unclear",
                            "summary": clause.get("summary")
                            or "the reviewer could not tell from the governed files",
                            "evidence": "; ".join(clause.get("evidence", [])) or None,
                            "fix": clause.get("fix"),
                            "source": "review",
                        }
                    )
                    continue
                findings.append(
                    {
                        "spec": entry["path"],
                        "clause": clause.get("clause"),
                        "severity": clause.get("severity")
                        or VERDICT_SEVERITY.get(verdict, ERROR),
                        "kind": verdict,
                        "summary": clause.get("summary") or "(no summary)",
                        "evidence": "; ".join(clause.get("evidence", [])) or None,
                        "failure": clause.get("failure"),
                        "fix": clause.get("fix"),
                        "source": "review",
                    }
                )
            for missing in sorted(expected):
                findings.append(
                    {
                        "spec": entry["path"],
                        "clause": missing,
                        "severity": ERROR,
                        "kind": "review-incomplete",
                        "summary": "the reviewer returned no verdict for this clause",
                        "evidence": str(path),
                        "source": "review",
                    }
                )
    return findings, reviewed


def render(findings, manifest, reviewed):
    lines = []
    by_spec = {}
    for item in findings:
        by_spec.setdefault(item["spec"], []).append(item)

    order = {ERROR: 0, WARNING: 1}
    for spec in sorted(by_spec):
        lines.append("")
        lines.append(spec)
        for item in sorted(by_spec[spec], key=lambda f: order.get(f["severity"], 2)):
            mark = "error" if item["severity"] == ERROR else "warn "
            clause = f"{item['clause']}: " if item.get("clause") else ""
            lines.append(f"  {mark}  {clause}{item['summary']}")
            if item.get("failure"):
                lines.append(f"         failure: {item['failure']}")
            if item.get("evidence"):
                lines.append(f"         at {item['evidence']}")
            if item.get("fix"):
                lines.append(f"         fix: {item['fix']}")

    errors = [f for f in findings if f["severity"] == ERROR]
    warnings = [f for f in findings if f["severity"] == WARNING]
    clauses = sum(entry["clauses"] for entry in manifest["specs"])
    lines.append("")
    lines.append(
        f"{len(manifest['specs'])} specs, {clauses} clauses, {reviewed} reviewed: "
        f"{len(errors)} errors, {len(warnings)} warnings"
    )
    if not errors and not warnings:
        lines.append("the implementation matches the spec")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-dir", required=True)
    parser.add_argument("--strict", action="store_true", help="warnings fail too")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    manifest_path = Path(args.work_dir) / "manifest.json"
    if not manifest_path.exists():
        print(f"no manifest at {manifest_path}", file=sys.stderr)
        return 2
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    findings = list(manifest.get("mechanical_findings", []))
    reviews, reviewed = load_reviews(manifest)
    findings.extend(reviews)

    strict = args.strict or manifest.get("strict", False)
    failed = any(f["severity"] == ERROR for f in findings) or (strict and bool(findings))

    if args.json:
        print(json.dumps({"findings": findings, "failed": failed}, indent=2))
    else:
        print(render(findings, manifest, reviewed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
