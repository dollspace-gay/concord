#!/usr/bin/env python3
"""Check split generator imports and transitive source provenance."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from qualification.load_recovery.provenance import generator_fingerprint

SCRIPTS = Path(__file__).resolve().parent


class GeneratorModuleTests(unittest.TestCase):
    def test_all_modules_import_without_starting_the_workload(self) -> None:
        environment = dict(os.environ)
        for name in list(environment):
            if name.startswith("CONCORD_QUAL_") or name == "CONCORD_QUALIFICATION_MODE":
                del environment[name]
        environment.update({
            "PYTHONPATH": str(SCRIPTS),
            "CONCORD_QUAL_IRC_HOST": "localhost", "CONCORD_QUAL_IRC_PORT": "1",
            "CONCORD_QUAL_HTTP_ORIGIN": "http://localhost", "CONCORD_QUALIFICATION_MODE": "smoke",
            "CONCORD_QUAL_SESSIONS": "1", "CONCORD_QUAL_SENDERS": "1", "CONCORD_QUAL_MESSAGES": "1",
            "CONCORD_QUAL_DURATION_SECONDS": "1", "CONCORD_QUAL_EVIDENCE_DIR": tempfile.gettempdir(),
            "CONCORD_QUAL_METRICS_SESSION": "isolated-fixture",
        })
        result = subprocess.run(
            [sys.executable, "-c", "import qualification.load_recovery.generator.runner"],
            env=environment, capture_output=True, text=True, timeout=10, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_every_generator_source_participates_in_the_fingerprint(self) -> None:
        with tempfile.TemporaryDirectory(prefix="concord-generator-provenance-") as directory:
            root = Path(directory)
            shutil.copy2(SCRIPTS / "load-recovery-generator.py", root)
            shutil.copytree(SCRIPTS / "qualification", root / "qualification",
                            ignore=shutil.ignore_patterns("__pycache__"))
            baseline = generator_fingerprint(root)
            sources = [root / "load-recovery-generator.py", root / "qualification/load_recovery/provenance.py",
                       *sorted((root / "qualification/load_recovery/generator").glob("*.py"))]
            for source in sources:
                with self.subTest(source=source.name):
                    original = source.read_bytes()
                    source.write_bytes(original + b"\n# mutation\n")
                    self.assertNotEqual(generator_fingerprint(root), baseline)
                    source.write_bytes(original)
                    self.assertEqual(generator_fingerprint(root), baseline)
            extra = root / "qualification/load_recovery/generator/extra.py"
            extra.write_text("# new dependency\n")
            self.assertNotEqual(generator_fingerprint(root), baseline)
            extra.unlink()
            cache = extra.parent / "__pycache__"
            cache.mkdir()
            (cache / "worker.pyc").write_bytes(b"compiled cache")
            self.assertEqual(generator_fingerprint(root), baseline)


if __name__ == "__main__":
    unittest.main()
