"""Constants for load/recovery evidence."""

from __future__ import annotations

GIB = 1024 * 1024 * 1024

MIB = 1024 * 1024

ACTIVE_COUNTS = ("connections", "jobs", "uploads")

SELF_TEST_QUERY_PLAN = b'[{"expected_total": 50, "query": "fixture"}]\n'
