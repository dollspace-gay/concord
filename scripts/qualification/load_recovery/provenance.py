"""Fingerprint the generator entry point and every transitive local source file."""
from __future__ import annotations

import hashlib
from pathlib import Path


def generator_fingerprint(scripts_root: Path | None = None) -> str:
    root = scripts_root or Path(__file__).resolve().parents[2]
    generator = root / "qualification/load_recovery/generator"
    files = [
        root / "load-recovery-generator.py",
        root / "qualification/__init__.py",
        root / "qualification/load_recovery/__init__.py",
        root / "qualification/load_recovery/provenance.py",
        *sorted(generator.rglob("*.py")),
    ]
    if not generator.is_dir():
        raise OSError(f"generator source directory is missing: {generator}")
    digest = hashlib.sha256()
    for path in files:
        name = path.relative_to(root).as_posix().encode()
        source = path.read_bytes()
        digest.update(len(name).to_bytes(8, "big"))
        digest.update(name)
        digest.update(len(source).to_bytes(8, "big"))
        digest.update(source)
    return digest.hexdigest()
