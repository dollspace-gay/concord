#!/usr/bin/env python3
"""Invoke the prepared target control command through a fixed SSH boundary."""

from __future__ import annotations

import os
import re
import shlex
import subprocess
import sys


SAFE_ARGUMENT = re.compile(r"^[A-Za-z0-9@._:/=-]+$")
ALLOWED_ACTIONS = {
    "restart",
    "provider-status",
    "provider-arm",
    "provider-disarm",
    "restore",
    "shutdown",
}


def main() -> int:
    target = os.environ.get("CONCORD_QUAL_SSH_TARGET", "")
    target_root = os.environ.get("CONCORD_QUAL_REMOTE_TARGET_ROOT", "")
    if not target or not target_root.startswith("/"):
        print("SSH target control environment is incomplete", file=sys.stderr)
        return 64
    if len(sys.argv) < 2 or sys.argv[1] not in ALLOWED_ACTIONS:
        print("unsupported qualification control action", file=sys.stderr)
        return 64
    arguments = sys.argv[1:]
    values = (target, target_root, *arguments)
    if any(len(value) > 512 or not SAFE_ARGUMENT.fullmatch(value) for value in values):
        print("qualification control argument is invalid", file=sys.stderr)
        return 64
    remote_control = f"{target_root}/bin/target-control"
    command = " ".join(
        shlex.quote(value)
        for value in (
            "env",
            f"CONCORD_QUAL_TARGET_ROOT={target_root}",
            remote_control,
            *arguments,
        )
    )
    completed = subprocess.run(
        ["ssh", "-T", "--", target, command],
        stdin=subprocess.DEVNULL,
        check=False,
    )
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
