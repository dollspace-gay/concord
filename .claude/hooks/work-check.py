#!/usr/bin/env python3
"""
PreToolUse hook that blocks Write|Edit|Bash unless a chainlink issue
is being actively worked on. Forces issue creation before code changes.
"""

import json
import subprocess
import sys
import os
import io

# Fix Windows encoding issues
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8')

# Git commands that are PERMANENTLY blocked — never allowed regardless of issue state
BLOCKED_GIT_COMMANDS = [
    "git push", "git commit", "git merge", "git rebase", "git cherry-pick",
    "git reset", "git checkout .", "git restore .", "git clean",
    "git stash", "git tag", "git am", "git apply",
    "git branch -d", "git branch -D", "git branch -m",
]

# Bash commands that are always allowed (read-only / chainlink management)
ALLOWED_BASH_PREFIXES = [
    "chainlink ",
    "git status", "git diff", "git log", "git branch", "git show",
    "cargo test", "cargo build", "cargo check", "cargo clippy", "cargo fmt",
    "npm test", "npm run", "npx ",
    "tsc", "node ", "python ",
    "ls", "dir", "pwd", "echo",
]


def find_chainlink_dir():
    """Find the .chainlink directory by walking up from cwd."""
    current = os.getcwd()
    for _ in range(10):
        candidate = os.path.join(current, '.chainlink')
        if os.path.isdir(candidate):
            return candidate
        parent = os.path.dirname(current)
        if parent == current:
            break
        current = parent
    return None


def run_chainlink(args):
    """Run a chainlink command and return output."""
    try:
        result = subprocess.run(
            ["chainlink"] + args,
            capture_output=True,
            text=True,
            timeout=3
        )
        return result.stdout.strip() if result.returncode == 0 else None
    except (subprocess.TimeoutExpired, FileNotFoundError, Exception):
        return None


def is_blocked_git(input_data):
    """Check if a Bash command is a blocked git mutation. Always denied."""
    command = input_data.get("tool_input", {}).get("command", "").strip()
    for blocked in BLOCKED_GIT_COMMANDS:
        if command.startswith(blocked):
            return True
    # Also catch piped/chained git mutations: && git push, ; git commit, etc.
    for blocked in BLOCKED_GIT_COMMANDS:
        if f"&& {blocked}" in command or f"; {blocked}" in command or f"| {blocked}" in command:
            return True
    return False


def is_allowed_bash(input_data):
    """Check if a Bash command is on the allow list (read-only/infra)."""
    command = input_data.get("tool_input", {}).get("command", "").strip()
    for prefix in ALLOWED_BASH_PREFIXES:
        if command.startswith(prefix):
            return True
    return False


def main():
    try:
        input_data = json.load(sys.stdin)
        tool_name = input_data.get('tool_name', '')
    except (json.JSONDecodeError, Exception):
        tool_name = ''

    # Only check on Write, Edit, Bash
    if tool_name not in ('Write', 'Edit', 'Bash'):
        sys.exit(0)

    # PERMANENT BLOCK: git mutation commands are never allowed
    if tool_name == 'Bash' and is_blocked_git(input_data):
        print(
            "DENIED: Git mutation commands are not allowed. "
            "Commits, pushes, merges, rebases, and other git write operations "
            "are performed by the human, not the AI.\n\n"
            "Read-only git commands (status, diff, log, show, branch) are allowed."
        )
        sys.exit(2)

    # Allow read-only / infrastructure Bash commands through
    if tool_name == 'Bash' and is_allowed_bash(input_data):
        sys.exit(0)

    chainlink_dir = find_chainlink_dir()
    if not chainlink_dir:
        sys.exit(0)

    # Check session status
    status = run_chainlink(["session", "status"])
    if not status:
        # chainlink not available — don't block
        sys.exit(0)

    # If already working on an issue, allow
    if "Working on: #" in status:
        sys.exit(0)

    # BLOCK: no active work item
    print(
        "BLOCKED: No active chainlink issue. "
        "Create and work on an issue before making changes.\n\n"
        "  chainlink quick \"<describe your task>\" -p <priority> -l <label>\n\n"
        "Or pick an existing issue:\n"
        "  chainlink list -s open\n"
        "  chainlink session work <id>"
    )
    sys.exit(2)


if __name__ == "__main__":
    main()
