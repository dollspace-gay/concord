"""Fixtures for load/recovery evidence."""

from __future__ import annotations
from ..provenance import generator_fingerprint
import hashlib
import json
from pathlib import Path
from typing import Any, Callable
from .constants import GIB, MIB, SELF_TEST_QUERY_PLAN

def valid_summary(*, mode: str = "full") -> dict[str, Any]:
    full = mode == "full"
    irc_sessions = 800 if full else 2
    web_sessions = 200 if full else 1
    messages_sent = 72_000 if full else 8
    duration = 3_600.0 if full else 8.0
    expected_deliveries = messages_sent * (100 if full else 2)
    return {
        "classification": "full-external-host-candidate" if full else "bounded-local-smoke",
        "acceptance_status": "full-acceptance-candidate" if full else "bounded-local-smoke",
        "client_errors": [],
        "unverified_acceptance_areas": [] if full else ["dedicated full host"],
        "full_acceptance_claimed": full,
        "inputs": {
            "server_sha256": "a" * 64,
            "generator_sha256": generator_fingerprint(),
            "seed_sha256": "d" * 64,
            "source_revision": "0123456789abcdef",
            "dataset_sha256": "b" * 64,
            "config_sha256": "e" * 64,
            "query_mix_sha256": hashlib.sha256(SELF_TEST_QUERY_PLAN).hexdigest(),
            "toolchain": "rustc test",
            "release_flags": "--release --locked",
        },
        "host": {
            "server_hostname": "target-host",
            "generator_hostname": "generator-host" if full else "target-host",
            "server_platform": "Linux test-target",
            "generator_platform": "Linux test-generator",
            "cpu_count": 4 if full else 2,
            "memory_bytes": 8 * GIB if full else 2 * GIB,
            "filesystem": "ext4",
            "storage": "local-ssd",
            "separate_load_generator": full,
        },
        "workload": {
            "irc_sessions": irc_sessions,
            "web_sessions": web_sessions,
            "sessions": irc_sessions + web_sessions,
            "servers": 50 if full else 1,
            "senders": 1,
            "seeded_messages": 1_000_000 if full else 100,
            "messages_sent": messages_sent,
            "duration_seconds": duration,
            "accepted_message_rate_per_second": messages_sent / duration,
            "mean_fanout": 100 if full else 2,
            "database_profile": "FULL/WAL",
        },
        "latency": {"commit_ack_p95_ms": 100.0, "commit_ack_p99_ms": 400.0},
        "exact_fanout": {
            "passed": True,
            "sent": messages_sent,
            "acked": messages_sent,
            "verified": messages_sent,
            "expected_deliveries": expected_deliveries,
            "observed_deliveries": expected_deliveries,
            "duplicates": 0,
            "missing": 0,
            "minimum_fanout": 100 if full else 2,
            "required_minimum_fanout": 100 if full else 2,
            "raw_replay_overlap_deduplicated": 0,
        },
        "history_search": {
            "passed": True,
            "history_requests": 5,
            "search_requests": 5,
            "history_p95_ms": 200.0,
            "search_p95_ms": 200.0,
            "authorization_failures": 0,
            "stable_pagination_checks": 5,
        },
        "reconnect": {
            "passed": True,
            "declared_target": 200 if full else 1,
            "recovered": 200 if full else 1,
            "convergence_seconds": 20.0,
            "duplicates": 0,
            "gaps": 0,
        },
        "slow_abusive": {
            "passed": True,
            "slow_clients": 100 if full else 1,
            "closed_or_resynced": 100 if full else 1,
            "healthy_p95_ms": 100.0,
            "healthy_p99_ms": 400.0,
            "permanently_blocked_workers": 0,
            "abusive": {
                "fragmented_command_latency_ms": 10.0,
                "invalid_command_code": "INVALID_INPUT",
                "oversized_command_code": "INVALID_INPUT",
                "rate_limit_code": "RATE_LIMITED",
                "connection_recovered_after_rate_limit": True,
            },
            "permission_race": {
                "concurrent_probe_count": 2,
                "concurrent_statuses": [200, 403],
                "post_revocation_status": 403,
            },
        },
        "media_provider": {
            "passed": True,
            "file_bytes_each": 100 * MIB if full else MIB,
            "concurrent_uploads": 4,
            "uploads_completed": 4,
            "provider_failure": True,
            "provider_failures_terminal": full,
            "core_chat_acks_during_failure": 1,
            "upload": {
                "all_four_overlapped": True,
                "chat_acknowledged_while_four_admitted": True,
                "retained_cursor_present": True,
                "chat_ack_latency_ms": 10.0,
                "attachments_cleaned_up": 4,
                "concurrent_uploads": 4,
                "file_bytes_each": 100 * MIB if full else MIB,
                "maximum_duration_seconds": 1.0,
            },
            "provider": {
                "chat_acknowledged": True,
                "retained_cursor_present": True,
                "terminal_required": full,
                "status": {
                    "delivery_error": "webhook_transport_unavailable",
                    "job_error": "webhook_transport_unavailable",
                    "last_status": None,
                    "delivery_state": "failed" if full else "pending",
                    "job_state": "failed" if full else "pending",
                    "delivery_attempts": 8 if full else 1,
                    "job_attempts": 8 if full else 1,
                },
            },
        },
        "restart_restore": {
            "passed": True,
            "restart": True,
            "restore": True,
            "restart_recovered": True,
            "restore_recovered": True,
            "accepted_messages_verified": True,
            "duplicate_publications": 0,
            "restart_result": {
                "ready_after_restart": True,
                "logical_message_count": 1,
                "no_duplicate_accepted_messages": True,
                "retained_cursor_resumed": True,
            },
            "restore_result": {
                "restored_main_messages": messages_sent,
                "restored_pending_jobs_reconciled": 1,
                "restored_provider_messages": 1,
                "restored_media_stress_messages": 1,
                "restored_restart_messages": 1,
                "old_session_invalidated": True,
                "integrity": "ok",
                "foreign_key_violations": 0,
                "missing_media": 0,
                "external_jobs_paused": True,
                "duplicate_publications": 0,
                "accepted_messages_verified": True,
                "declared_restore_point": "fixture-point",
            },
        },
        "no_duplicate_accepted_messages": True,
        "resource": {"scope": "full" if mode == "full" else "bounded"},
    }

def valid_samples() -> list[dict[str, Any]]:
    samples: list[dict[str, Any]] = []
    for index in range(5):
        samples.append(
            {
                "unix_seconds": float(index),
                "phase": "warmup",
                "ready_status": 200,
                "concord_metrics_status": 200,
                "server_telemetry": {
                    "rss_bytes": (100 + index) * MIB,
                    "connections": 0,
                    "jobs": 0,
                    "uploads": 0,
                },
            }
        )
    for index in range(61):
        samples.append(
            {
                "unix_seconds": 5.0 + index * 60.0,
                "phase": "steady",
                "ready_status": 200,
                "concord_metrics_status": 200,
                "server_telemetry": {
                    "rss_bytes": (110 + min(index, 20)) * MIB,
                    "connections": 1_000,
                    "jobs": 2,
                    "uploads": 1,
                },
            }
        )
    for index in range(4):
        samples.append(
            {
                "unix_seconds": 3_606.0 + index,
                "phase": "stress",
                "ready_status": 200,
                "concord_metrics_status": 200,
                "server_telemetry": {
                    "rss_bytes": (140 + index) * MIB,
                    "connections": 1,
                    "jobs": 1,
                    "uploads": 4 if index < 2 else 0,
                },
            }
        )
    for index in range(5):
        samples.append(
            {
                "unix_seconds": 3_610.0 + index,
                "phase": "post_disconnect",
                "ready_status": 200,
                "concord_metrics_status": 200,
                "server_telemetry": {
                    "rss_bytes": (105 - index) * MIB,
                    "connections": 0,
                    "jobs": 0,
                    "uploads": 0,
                },
            }
        )
    return samples

def write_fixture(
    root: Path, summary: dict[str, Any], samples: list[dict[str, Any]]
) -> None:
    root.mkdir(parents=True, exist_ok=True)
    (root / "summary.json").write_text(
        json.dumps(summary, allow_nan=False) + "\n", encoding="utf-8"
    )
    (root / "time-series.jsonl").write_text(
        "".join(json.dumps(sample, allow_nan=False) + "\n" for sample in samples),
        encoding="utf-8",
    )
    if summary.get("full_acceptance_claimed") is True:
        host = summary["host"]
        inputs = summary["inputs"]
        workload = summary["workload"]
        metadata = {
            "hostname": host["server_hostname"],
            "kernel": host["server_platform"],
            "filesystem": host["filesystem"],
            "storage": host["storage"],
            "cpu_count": host["cpu_count"],
            "memory_bytes": host["memory_bytes"],
            "server_sha256": inputs["server_sha256"],
            "seed_sha256": inputs["seed_sha256"],
            "dataset_sha256": inputs["dataset_sha256"],
            "config_sha256": inputs["config_sha256"],
            "query_mix_sha256": inputs["query_mix_sha256"],
            "source_revision": inputs["source_revision"],
            "rustc": inputs["toolchain"],
            "release_flags": inputs["release_flags"],
            "database_profile": workload["database_profile"],
            "seeded_messages": workload["seeded_messages"],
            "configured_max_upload_bytes": summary["media_provider"]["file_bytes_each"],
        }
        (root / "server-metadata.json").write_text(
            json.dumps(metadata) + "\n", encoding="utf-8"
        )
        (root / "query-plan.json").write_bytes(SELF_TEST_QUERY_PLAN)
        (root / "channel-inventory.json").write_text(
            json.dumps([f"#server-{index}" for index in range(50)]) + "\n",
            encoding="utf-8",
        )
        (root / "credential-inventory.redacted.json").write_text(
            json.dumps({"credential_count": workload["irc_sessions"] + workload["senders"]})
            + "\n",
            encoding="utf-8",
        )
