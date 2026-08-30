"""Reading docs/specs: front matter, clauses, and the tests a clause names.

Shared by check-spec.py and collect-findings.py so that the mechanical pass and the
report agree on what a spec is. Standard library only, in keeping with the
repository's rule about dependencies.
"""

import re
from pathlib import Path

SPEC_DIR = Path("docs/specs")
README = SPEC_DIR / "README.md"

CLAUSE_HEADING = re.compile(r"^###\s+([A-Z]+)-(\d+)\s*:\s*(.*)$")
VERIFIED_BY = re.compile(r"verified-by:\s*([^`\n]+)")
TEST_FN = re.compile(r"^\s*fn\s+([A-Za-z0-9_]+)\s*\(")
EM_DASH = "—"


class Clause:
    def __init__(self, spec, prefix, number, title, line):
        self.spec = spec
        self.prefix = prefix
        self.number = number
        self.title = title
        self.line = line
        self.body = []
        self.verified_by = []

    @property
    def id(self):
        return f"{self.prefix}-{self.number}"

    @property
    def text(self):
        return "\n".join(self.body)

    @property
    def withdrawn(self):
        return "withdrawn" in self.title.lower()

    def as_dict(self):
        return {
            "id": self.id,
            "title": self.title,
            "line": self.line,
            "verified_by": self.verified_by,
            "withdrawn": self.withdrawn,
        }


class Spec:
    def __init__(self, path):
        self.path = path
        self.rel = str(path)
        self.name = path.name
        self.front = {}
        self.front_lines = {}
        self.clauses = []
        self.lines = []
        self.malformed_front_matter = False

    @property
    def id(self):
        return self.front.get("id", "")

    @property
    def governs(self):
        return self.front.get("governs", [])

    @property
    def guards(self):
        return self.front.get("guards", [])

    @property
    def commentary(self):
        """Everything outside a clause. Binds nobody, but still gets read."""
        clause_lines = {c.line for c in self.clauses}
        if not clause_lines:
            return "\n".join(self.lines)
        first = min(clause_lines)
        return "\n".join(self.lines[: first - 1])


def _parse_front_matter(lines):
    """The small YAML subset the specs actually use: scalars, and lists of either
    plain strings or a single `symbol:` key. A real parser is not worth a dependency,
    and anything outside that subset is reported rather than guessed at."""
    if not lines or lines[0].strip() != "---":
        return None, 0
    end = None
    for i in range(1, len(lines)):
        if lines[i].strip() == "---":
            end = i
            break
    if end is None:
        return None, 0

    front = {}
    key = None
    for raw in lines[1:end]:
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        if raw.startswith(("  - ", "- ")):
            item = raw.split("- ", 1)[1].strip()
            if item.startswith("symbol:"):
                item = item.split("symbol:", 1)[1].strip()
            if key is None:
                return None, end + 1
            front.setdefault(key, [])
            if not isinstance(front[key], list):
                return None, end + 1
            front[key].append(item)
            continue
        if ":" not in raw:
            return None, end + 1
        key, value = raw.split(":", 1)
        key = key.strip()
        value = value.strip()
        front[key] = value if value else []
    return front, end + 1


def load_spec(path):
    spec = Spec(path)
    text = path.read_text(encoding="utf-8")
    spec.lines = text.split("\n")
    front, _ = _parse_front_matter(spec.lines)
    if front is None:
        spec.malformed_front_matter = True
    else:
        spec.front = front

    current = None
    for number, raw in enumerate(spec.lines, start=1):
        heading = CLAUSE_HEADING.match(raw)
        if heading:
            current = Clause(spec, heading.group(1), int(heading.group(2)), heading.group(3), number)
            spec.clauses.append(current)
            continue
        if current is None:
            continue
        if raw.startswith("## ") or raw.startswith("### "):
            current = None
            continue
        current.body.append(raw)
        found = VERIFIED_BY.search(raw)
        if found:
            current.verified_by.append(found.group(1).strip().rstrip("`").strip())
    return spec


def load_specs(root=SPEC_DIR):
    paths = sorted(p for p in root.rglob("*.md") if p.name != "README.md")
    return [load_spec(p) for p in paths]


def crate_directories():
    """Package name to crate directory, so `bravebot_core::label::x` finds crates/core."""
    mapping = {}
    for manifest in sorted(Path("crates").glob("*/Cargo.toml")):
        for line in manifest.read_text(encoding="utf-8").split("\n"):
            if line.startswith("name"):
                name = line.split("=", 1)[1].strip().strip('"')
                mapping[name.replace("-", "_")] = manifest.parent
                break
    return mapping


class TestIndex:
    """Every `#[test]` function in the workspace, by name and by file."""

    def __init__(self):
        self.by_name = {}
        for source in sorted(Path("crates").rglob("*.rs")):
            lines = source.read_text(encoding="utf-8", errors="replace").split("\n")
            attributed = False
            for number, raw in enumerate(lines, start=1):
                stripped = raw.strip()
                if stripped == "#[test]":
                    attributed = True
                    continue
                if not attributed:
                    continue
                name = TEST_FN.match(raw)
                if name:
                    self.by_name.setdefault(name.group(1), []).append((source, number))
                    attributed = False
                elif stripped and not stripped.startswith("#["):
                    attributed = False

    def candidates(self, crate_dir, module_path):
        """Where a `crate::module::test` reference says the test should live."""
        joined = "/".join(module_path)
        if not joined:
            return []
        return [
            crate_dir / "src" / f"{joined}.rs",
            crate_dir / "src" / joined / "mod.rs",
            crate_dir / "tests" / f"{joined}.rs",
        ]

    def resolve(self, reference, crates):
        """Resolve one `verified-by` value.

        Returns (status, detail) where status is one of `ok`, `moved`, `missing`,
        `unknown-crate`, or `malformed`.
        """
        parts = reference.split("::")
        if len(parts) < 2:
            return "malformed", "expected crate::module::test_name"
        crate_dir = crates.get(parts[0])
        if crate_dir is None:
            return "unknown-crate", parts[0]
        name = parts[-1]
        sites = self.by_name.get(name)
        if not sites:
            return "missing", name
        expected = self.candidates(crate_dir, parts[1:-1])
        for path, line in sites:
            if path in expected:
                return "ok", f"{path}:{line}"
        found = ", ".join(f"{path}:{line}" for path, line in sites)
        return "moved", found
