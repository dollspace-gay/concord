#!/usr/bin/env python3
"""Validate Concord load/recovery evidence without exposing fixture secrets."""
from qualification.load_recovery.analysis.cli import main

if __name__ == "__main__":
    raise SystemExit(main())
