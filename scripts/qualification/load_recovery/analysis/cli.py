"""Validate Concord load/recovery evidence without exposing fixture secrets."""

from __future__ import annotations
import argparse
from pathlib import Path
import sys
from .validation import AnalysisError
from .analyze import analyze
from .self_test import run_self_test

def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("smoke", "full"))
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        if args.mode is not None or args.evidence is not None:
            parser.error("--self-test cannot be combined with --mode or --evidence")
    elif args.mode is None or args.evidence is None:
        parser.error("--mode and --evidence are required unless --self-test is used")
    return args

def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.self_test:
        return run_self_test()
    try:
        result = analyze(args.mode, args.evidence)
    except (AnalysisError, OSError) as error:
        print(f"FAIL load-recovery-evidence: {error}", file=sys.stderr)
        return 1
    analysis_path = args.evidence / "analysis.json"
    print(
        "PASS load-recovery-evidence "
        f"mode={args.mode} classification={result['classification']} "
        f"analysis={analysis_path}"
    )
    return 0
