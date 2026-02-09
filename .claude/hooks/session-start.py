#!/usr/bin/env python3
"""
Session start hook that loads chainlink context and auto-starts sessions.
"""

import json
import re
import subprocess
import sys
import os
from datetime import datetime, timezone


# Sessions older than this (in hours) are considered stale and auto-ended
STALE_SESSION_HOURS = 4


def run_chainlink(args):
    """Run a chainlink command and return output."""
    try:
        result = subprocess.run(
            ["chainlink"] + args,
            capture_output=True,
            text=True,
            timeout=5
        )
        return result.stdout.strip() if result.returncode == 0 else None
    except (subprocess.TimeoutExpired, FileNotFoundError, Exception):
        return None


def check_chainlink_initialized():
    """Check if .chainlink directory exists."""
    cwd = os.getcwd()
    current = cwd

    while True:
        candidate = os.path.join(current, ".chainlink")
        if os.path.isdir(candidate):
            return True
        parent = os.path.dirname(current)
        if parent == current:
            break
        current = parent

    return False


def get_session_age_minutes():
    """Parse session status to get duration in minutes. Returns None if no active session."""
    result = run_chainlink(["session", "status"])
    if not result or "Session #" not in result:
        return None
    match = re.search(r'Duration:\s*(\d+)\s*minutes', result)
    if match:
        return int(match.group(1))
    return None


def has_active_session():
    """Check if there's an active chainlink session."""
    result = run_chainlink(["session", "status"])
    if result and "Session #" in result and "(started" in result:
        return True
    return False


def auto_end_stale_session():
    """End session if it's been open longer than STALE_SESSION_HOURS."""
    age_minutes = get_session_age_minutes()
    if age_minutes is not None and age_minutes > STALE_SESSION_HOURS * 60:
        run_chainlink([
            "session", "end", "--notes",
            f"Session auto-ended (stale after {age_minutes} minutes). No handoff notes provided."
        ])
        return True
    return False


def detect_resume_event():
    """Detect if this is a resume (context compression) vs fresh startup.

    If there's already an active session, this is a resume event.
    """
    return has_active_session()


def get_last_action_from_status(status_text):
    """Extract last action from session status output."""
    if not status_text:
        return None
    match = re.search(r'Last action:\s*(.+)', status_text)
    if match:
        return match.group(1).strip()
    return None


def auto_comment_on_resume(session_status):
    """Add auto-comment on active issue when resuming after context compression."""
    if not session_status:
        return
    # Extract working issue ID
    match = re.search(r'Working on: #(\d+)', session_status)
    if not match:
        return
    issue_id = match.group(1)

    last_action = get_last_action_from_status(session_status)
    if last_action:
        comment = f"[auto] Session resumed after context compression. Last action: {last_action}"
    else:
        comment = "[auto] Session resumed after context compression."

    run_chainlink(["comment", issue_id, comment])


def main():
    if not check_chainlink_initialized():
        # No chainlink repo, skip
        sys.exit(0)

    context_parts = ["<chainlink-session-context>"]

    is_resume = detect_resume_event()

    # Check for stale session and auto-end it
    stale_ended = False
    if is_resume:
        stale_ended = auto_end_stale_session()
        if stale_ended:
            is_resume = False
            context_parts.append(
                "## Stale Session Warning\nPrevious session was auto-ended (open > "
                f"{STALE_SESSION_HOURS} hours). Handoff notes may be incomplete."
            )

    # Get handoff notes from previous session before starting new one
    last_handoff = run_chainlink(["session", "last-handoff"])

    # Auto-start session if none active
    if not has_active_session():
        run_chainlink(["session", "start"])

    # If resuming, add breadcrumb comment and context
    if is_resume:
        session_status = run_chainlink(["session", "status"])
        auto_comment_on_resume(session_status)

        last_action = get_last_action_from_status(session_status)
        if last_action:
            context_parts.append(
                f"## Context Compression Breadcrumb\n"
                f"This session resumed after context compression.\n"
                f"Last recorded action: {last_action}"
            )
        else:
            context_parts.append(
                "## Context Compression Breadcrumb\n"
                "This session resumed after context compression.\n"
                "No last action was recorded. Use `chainlink session action \"...\"` to track progress."
            )

    # Include previous session handoff notes if available
    if last_handoff and "No previous" not in last_handoff:
        context_parts.append(f"## Previous Session Handoff\n{last_handoff}")

    # Try to get session status
    session_status = run_chainlink(["session", "status"])
    if session_status:
        context_parts.append(f"## Current Session\n{session_status}")

    # Get ready issues (unblocked work)
    ready_issues = run_chainlink(["ready"])
    if ready_issues:
        context_parts.append(f"## Ready Issues (unblocked)\n{ready_issues}")

    # Get open issues summary
    open_issues = run_chainlink(["list", "-s", "open"])
    if open_issues:
        context_parts.append(f"## Open Issues\n{open_issues}")

    context_parts.append("""
## Chainlink Workflow Reminder
- Use `chainlink session start` at the beginning of work
- Use `chainlink session work <id>` to mark current focus
- Use `chainlink session action "..."` to record breadcrumbs before context compression
- Add comments as you discover things: `chainlink comment <id> "..."`
- End with handoff notes: `chainlink session end --notes "..."`
</chainlink-session-context>""")

    print("\n\n".join(context_parts))
    sys.exit(0)


if __name__ == "__main__":
    main()
