#!/usr/bin/env python3
"""Check the soft source-file budget and local agent guidance."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

SOURCE_ROOTS = ("concord/server", "concord/web", "scripts")
SOURCE_SUFFIXES = {".rs", ".c", ".h", ".ts", ".tsx", ".js", ".mjs", ".py", ".sh", ".sql", ".css"}
EXCLUDED_PARTS = {"node_modules", "target", "dist", "test-results", "playwright-report", "__pycache__"}
GENERATED_ROOT = Path("concord/web/src/api/generated")
SOFT_LIMIT = 500
EXCEPTIONS = Path(".design/maintainability/size-exceptions.json")


def source_files(root: Path) -> list[Path]:
    found = []
    for relative in SOURCE_ROOTS:
        directory = root / relative
        for path in directory.rglob("*"):
            if not path.is_file() or EXCLUDED_PARTS.intersection(path.relative_to(root).parts):
                continue
            if path.relative_to(root).is_relative_to(GENERATED_ROOT):
                continue
            if path.suffix in SOURCE_SUFFIXES or path.parent == root / "scripts/suites":
                found.append(path)
    return sorted(found)


def owned_directories(root: Path, files: list[Path]) -> set[Path]:
    directories = set()
    for path in files:
        parent = path.parent
        while parent != root:
            directories.add(parent)
            parent = parent.parent
    return directories


def inspect(root: Path) -> dict:
    files = source_files(root)
    exceptions = json.loads((root / EXCEPTIONS).read_text())
    if not isinstance(exceptions, dict):
        raise ValueError("size exceptions must be an object keyed by relative source path")
    violations = []
    reviewed = []
    counts = {path.relative_to(root).as_posix(): len(path.read_text().splitlines()) for path in files}
    for name, rule in exceptions.items():
        if name not in counts:
            violations.append(f"stale exception: {name} is not owned source")
            continue
        if not isinstance(rule, dict) or not isinstance(rule.get("reason"), str) or not rule["reason"].strip():
            violations.append(f"exception lacks a review reason: {name}")
            continue
        maximum = rule.get("maximum_lines")
        if type(maximum) is not int or maximum <= SOFT_LIMIT:
            violations.append(f"exception needs an integer maximum above {SOFT_LIMIT}: {name}")
            continue
        if counts[name] <= SOFT_LIMIT:
            violations.append(f"remove unnecessary exception: {name}")
        elif counts[name] > maximum:
            violations.append(f"{name}: {counts[name]} lines exceeds reviewed maximum {maximum}")
        else:
            reviewed.append({"path": name, "lines": counts[name], "reason": rule["reason"]})
    for name, count in counts.items():
        if count > SOFT_LIMIT and name not in exceptions:
            violations.append(f"{name}: {count} lines exceeds the soft {SOFT_LIMIT}-line budget; split by responsibility or record a reviewed exception")
    directories = owned_directories(root, files)
    for directory in sorted(directories):
        guidance = directory / "AGENTS.md"
        if not guidance.is_file() or not guidance.read_text().strip():
            violations.append(f"missing local guidance: {guidance.relative_to(root)}")
    return {
        "source_files": len(files),
        "guided_directories": len(directories),
        "soft_limit": SOFT_LIMIT,
        "reviewed_exceptions": reviewed,
        "violations": violations,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--json", action="store_true", help="print the complete review report")
    args = parser.parse_args()
    try:
        report = inspect(args.root.resolve())
    except (OSError, ValueError) as error:
        parser.exit(1, f"maintainability configuration failed: {error}\n")
    if args.json:
        print(json.dumps(report, indent=2))
    elif report["violations"]:
        print("maintainability review required:\n" + "\n".join(f"- {v}" for v in report["violations"]))
    else:
        print(f"maintainability passed: {report['source_files']} source files, {report['guided_directories']} guided directories, "
              f"{len(report['reviewed_exceptions'])} reviewed exceptions to the soft {SOFT_LIMIT}-line budget")
    return int(bool(report["violations"]))


if __name__ == "__main__":
    raise SystemExit(main())
