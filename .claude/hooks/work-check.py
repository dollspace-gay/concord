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

# Defaults — overridden by .chainlink/hook-config.json if present
DEFAULT_BLOCKED_GIT = [
    "git push", "git commit", "git merge", "git rebase", "git cherry-pick",
    "git reset", "git checkout .", "git restore .", "git clean",
    "git stash", "git tag", "git am", "git apply",
    "git branch -d", "git branch -D", "git branch -m",
]

DEFAULT_ALLOWED_BASH = [
    "chainlink ",
    "git status", "git diff", "git log", "git branch", "git show",
    "cargo test", "cargo build", "cargo check", "cargo clippy", "cargo fmt",
    "npm test", "npm run", "npx ",
    "tsc", "node ", "python ",
    "ls", "dir", "pwd", "echo",
]


def load_config(chainlink_dir):
    """Load hook config from .chainlink/hook-config.json, falling back to defaults.

    Returns (tracking_mode, blocked_git, allowed_bash).
    tracking_mode is one of: "strict", "normal", "relaxed".
      strict  — block Write/Edit/Bash without an active issue
      normal  — remind (print warning) but don't block
      relaxed — no issue-tracking enforcement, only git blocks
    """
    blocked = list(DEFAULT_BLOCKED_GIT)
    allowed = list(DEFAULT_ALLOWED_BASH)
    mode = "strict"

    if not chainlink_dir:
        return mode, blocked, allowed

    config_path = os.path.join(chainlink_dir, "hook-config.json")
    if not os.path.isfile(config_path):
        return mode, blocked, allowed

    try:
        with open(config_path, "r", encoding="utf-8") as f:
            config = json.load(f)

        if config.get("tracking_mode") in ("strict", "normal", "relaxed"):
            mode = config["tracking_mode"]
        if "blocked_git_commands" in config:
            blocked = config["blocked_git_commands"]
        if "allowed_bash_prefixes" in config:
            allowed = config["allowed_bash_prefixes"]
    except (json.JSONDecodeError, OSError):
        pass

    return mode, blocked, allowed


def _project_root_from_script():
    """Derive project root from this script's location (.claude/hooks/<script>.py -> project root)."""
    try:
        return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    except (NameError, OSError):
        return None


def find_chainlink_dir():
    """Find the .chainlink directory.

    Prefers the project root derived from the hook script's own path
    (reliable even when cwd is a subdirectory), falling back to walking
    up from cwd for standalone/test usage.
    """
    # Primary: resolve from script location
    root = _project_root_from_script()
    if root:
        candidate = os.path.join(root, '.chainlink')
        if os.path.isdir(candidate):
            return candidate

    # Fallback: walk up from cwd
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


def is_blocked_git(input_data, blocked_list):
    """Check if a Bash command is a blocked git mutation. Always denied."""
    command = input_data.get("tool_input", {}).get("command", "").strip()
    for blocked in blocked_list:
        if command.startswith(blocked):
            return True
    # Also catch piped/chained git mutations: && git push, ; git commit, etc.
    for blocked in blocked_list:
        if f"&& {blocked}" in command or f"; {blocked}" in command or f"| {blocked}" in command:
            return True
    return False


def is_allowed_bash(input_data, allowed_list):
    """Check if a Bash command is on the allow list (read-only/infra)."""
    command = input_data.get("tool_input", {}).get("command", "").strip()
    for prefix in allowed_list:
        if command.startswith(prefix):
            return True
    return False


def is_claude_memory_path(input_data):
    """Check if a Write/Edit targets Claude Code's own memory/config directory (~/.claude/)."""
    file_path = input_data.get("tool_input", {}).get("file_path", "")
    if not file_path:
        return False
    home = os.path.expanduser("~")
    claude_dir = os.path.join(home, ".claude")
    try:
        return os.path.normcase(os.path.abspath(file_path)).startswith(
            os.path.normcase(os.path.abspath(claude_dir))
        )
    except (ValueError, OSError):
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

    # Allow Claude Code to manage its own memory/config in ~/.claude/
    if tool_name in ('Write', 'Edit') and is_claude_memory_path(input_data):
        sys.exit(0)

    chainlink_dir = find_chainlink_dir()
    tracking_mode, blocked_git, allowed_bash = load_config(chainlink_dir)

    # PERMANENT BLOCK: git mutation commands are never allowed (all modes)
    if tool_name == 'Bash' and is_blocked_git(input_data, blocked_git):
        print(
            "DENIED: Git mutation commands are not allowed. "
            "Commits, pushes, merges, rebases, and other git write operations "
            "are performed by the human, not the AI.\n\n"
            "Read-only git commands (status, diff, log, show, branch) are allowed."
        )
        sys.exit(2)

    # Allow read-only / infrastructure Bash commands through
    if tool_name == 'Bash' and is_allowed_bash(input_data, allowed_bash):
        sys.exit(0)

    # Relaxed mode: no issue-tracking enforcement
    if tracking_mode == "relaxed":
        sys.exit(0)

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

    # No active work item — behavior depends on mode
    nudge_msg = (
        "No active chainlink issue. "
        "Create and work on an issue before making changes.\n\n"
        "  chainlink quick \"<describe your task>\" -p <priority> -l <label>\n\n"
        "Or pick an existing issue:\n"
        "  chainlink list -s open\n"
        "  chainlink session work <id>"
    )

    if tracking_mode == "strict":
        print(f"BLOCKED: {nudge_msg}")
        sys.exit(2)
    else:
        # normal mode: remind but allow
        print(f"Reminder: {nudge_msg}")
        sys.exit(0)


if __name__ == "__main__":
    main()
