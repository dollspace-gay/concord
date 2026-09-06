#!/usr/bin/env python3
"""Negative fixtures for the handwritten source budget and guidance policy."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("maintainability", ROOT / "scripts/check-maintainability.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class MaintainabilityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="concord-maintainability-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "concord/server/src/example.rs"
        self.source.parent.mkdir(parents=True)
        self.source.write_text("// source\n" * 500)
        for path in (self.root / "concord", self.root / "concord/server", self.source.parent):
            (path / "AGENTS.md").write_text("Local ownership and checks.\n")
        (self.root / checker.EXCEPTIONS).parent.mkdir(parents=True)
        self.set_exceptions({})

    def set_exceptions(self, value: object) -> None:
        (self.root / checker.EXCEPTIONS).write_text(json.dumps(value))

    def violations(self) -> list[str]:
        return checker.inspect(self.root)["violations"]

    def test_exact_budget_passes(self) -> None:
        self.assertEqual(self.violations(), [])

    def test_new_overage_requires_review(self) -> None:
        self.source.write_text("// source\n" * 501)
        self.assertIn("exceeds the soft", "\n".join(self.violations()))

    def test_reviewed_exception_cannot_silently_grow(self) -> None:
        self.set_exceptions({"concord/server/src/example.rs": {
            "maximum_lines": 501, "reason": "A reviewed indivisible declaration.",
        }})
        self.source.write_text("// source\n" * 501)
        self.assertEqual(self.violations(), [])
        self.source.write_text("// source\n" * 502)
        self.assertIn("reviewed maximum", "\n".join(self.violations()))

    def test_stale_or_unnecessary_exception_is_rejected(self) -> None:
        self.set_exceptions({"concord/server/src/missing.rs": {
            "maximum_lines": 501, "reason": "Old file.",
        }})
        self.assertIn("stale exception", "\n".join(self.violations()))
        self.set_exceptions({"concord/server/src/example.rs": {
            "maximum_lines": 501, "reason": "Now below the budget.",
        }})
        self.assertIn("unnecessary exception", "\n".join(self.violations()))

    def test_exception_needs_reason_and_integer_limit(self) -> None:
        self.set_exceptions({"concord/server/src/example.rs": {"maximum_lines": 501}})
        self.assertIn("review reason", "\n".join(self.violations()))
        self.set_exceptions({"concord/server/src/example.rs": {
            "maximum_lines": True, "reason": "Bad limit.",
        }})
        self.assertIn("integer maximum", "\n".join(self.violations()))

    def test_new_source_directory_needs_its_own_guidance(self) -> None:
        nested = self.source.parent / "example/worker.rs"
        nested.parent.mkdir()
        nested.write_text("fn work() {}\n")
        self.assertIn("example/AGENTS.md", "\n".join(self.violations()))
        (nested.parent / "AGENTS.md").write_text("Worker ownership.\n")
        self.assertEqual(self.violations(), [])

    def test_only_known_generated_and_build_trees_are_excluded(self) -> None:
        for relative in ("concord/web/src/api/generated/contract.ts", "concord/web/node_modules/lib/index.ts",
                         "concord/web/dist/assets/app.js", "concord/web/test-results/copied.ts"):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// generated\n" * 900)
        self.assertEqual(self.violations(), [])
        handwritten = self.source.parent / "generated.rs"
        handwritten.write_text("// handwritten\n" * 501)
        self.assertIn("generated.rs", "\n".join(self.violations()))


if __name__ == "__main__":
    unittest.main()
