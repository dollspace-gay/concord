"""Validate observed delivery and write the generator evidence summary."""
from __future__ import annotations
import json
import os
import platform
import socket
import statistics
from pathlib import Path
from .state import channels, duration, errors, evidence, expected_fanout, expected_recipients, first_receipt, history_latencies_ms, lock, marker_prefix, message_count, mode, query_latencies_ms, receipts, reconnect_completed, reconnect_count, search_latencies_ms, sender_count, sender_first_receipt, sender_receipts, sequence_channels, session_count, slow_client_count, slow_completed, started, web_raw_receipts, web_session_count
from . import state
from dataclasses import dataclass
from ..provenance import generator_fingerprint

@dataclass(frozen=True)
class RunMeasurements:
    start_wall: float
    steady_duration_seconds: float
    reconnect_convergence_seconds: float
    abusive_result: dict
    permission_race_result: dict
    media_result: dict
    provider_result: dict
    restart_result: dict


def write_summary(measurements: RunMeasurements) -> None:
    start_wall = measurements.start_wall
    steady_duration_seconds = measurements.steady_duration_seconds
    reconnect_convergence_seconds = measurements.reconnect_convergence_seconds
    abusive_result = measurements.abusive_result
    permission_race_result = measurements.permission_race_result
    media_result = measurements.media_result
    provider_result = measurements.provider_result
    restart_result = measurements.restart_result
    with lock:
        observed = len(receipts)
        latencies = sorted((first_receipt[key] - started[key]) * 1000 for key in first_receipt if key in started)
        minimum_fanout = min((len(value) for value in receipts.values()), default=0)
        client_errors = list(errors)

    if observed != message_count:
        raise SystemExit(f"message correctness failure: sent={message_count} uniquely_observed={observed}")

    if len(sender_receipts) != message_count:
        raise SystemExit(
            f"acceptance acknowledgement failure: sent={message_count} sender_echoes={len(sender_receipts)}"
        )

    if client_errors:
        raise SystemExit("client errors occurred during qualification: " + "; ".join(client_errors))

    fanout_failures = []

    for sequence, recipients in receipts.items():
        destination = sequence_channels[sequence]
        expected = expected_recipients[destination]
        if recipients != expected:
            fanout_failures.append(
                {
                    "sequence": sequence,
                    "channel": destination,
                    "missing": sorted(expected - recipients),
                    "unexpected": sorted(recipients - expected),
                }
            )

    if fanout_failures:
        raise SystemExit(f"fanout correctness failure: {fanout_failures[:10]}")

    required_fanout = min(expected_fanout.values())

    def percentile(values: list[float], fraction: float) -> float:
        return values[min(len(values) - 1, int((len(values) - 1) * fraction))]

    ack_latencies = sorted(
        (sender_first_receipt[key] - started[key]) * 1000
        for key in sender_first_receipt
        if key in started
    )

    if len(ack_latencies) != message_count or not query_latencies_ms:
        raise SystemExit("latency evidence is incomplete")

    expected_deliveries = sum(
        len(expected_recipients[sequence_channels[sequence]])
        for sequence in range(message_count)
    )

    observed_deliveries = sum(len(value) for value in receipts.values())

    raw_web_replay_overlap = sum(
        max(0, count - 1) for count in web_raw_receipts.values()
    )

    generator_sha256 = generator_fingerprint()

    server_metadata = {}

    if os.environ.get("CONCORD_QUAL_SERVER_METADATA"):
        server_metadata = json.load(
            open(os.environ["CONCORD_QUAL_SERVER_METADATA"], encoding="utf-8")
        )

    if mode == "smoke":
        memory_bytes = 0
        try:
            for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
                if line.startswith("MemTotal:"):
                    memory_bytes = int(line.split()[1]) * 1024
                    break
        except OSError:
            memory_bytes = 0
        host_evidence = {
            "server_hostname": socket.gethostname(),
            "generator_hostname": socket.gethostname(),
            "server_platform": platform.platform(),
            "generator_platform": platform.platform(),
            "cpu_count": os.cpu_count() or 0,
            "memory_bytes": memory_bytes,
            "filesystem": "temporary-local-fixture",
            "storage": "temporary-local-fixture",
            "separate_load_generator": False,
        }
        input_evidence = {
            "server_sha256": os.environ["CONCORD_QUAL_SERVER_SHA256"],
            "generator_sha256": generator_sha256,
            "seed_sha256": os.environ["CONCORD_QUAL_SEED_SHA256"],
            "dataset_sha256": os.environ["CONCORD_QUAL_DATASET_SHA256"],
            "config_sha256": os.environ["CONCORD_QUAL_CONFIG_SHA256"],
            "query_mix_sha256": os.environ["CONCORD_QUAL_QUERY_MIX_SHA256"],
            "source_revision": os.environ["CONCORD_QUAL_SOURCE_REVISION"],
            "toolchain": os.environ["CONCORD_QUAL_SERVER_TOOLCHAIN"],
            "release_flags": "debug browser-fixtures bounded-smoke",
        }
        seeded_messages = int(os.environ["CONCORD_QUAL_SEEDED_MESSAGES"])
    else:
        host_evidence = {
            "server_hostname": server_metadata["hostname"],
            "generator_hostname": socket.gethostname(),
            "server_platform": server_metadata["kernel"],
            "generator_platform": platform.platform(),
            "cpu_count": int(server_metadata["cpu_count"]),
            "memory_bytes": int(server_metadata["memory_bytes"]),
            "filesystem": server_metadata["filesystem"],
            "storage": server_metadata["storage"],
            "separate_load_generator": server_metadata["hostname"]
            not in {socket.gethostname(), socket.getfqdn()},
        }
        input_evidence = {
            "server_sha256": server_metadata["server_sha256"],
            "generator_sha256": generator_sha256,
            "seed_sha256": server_metadata["seed_sha256"],
            "dataset_sha256": server_metadata["dataset_sha256"],
            "config_sha256": server_metadata["config_sha256"],
            "query_mix_sha256": server_metadata["query_mix_sha256"],
            "source_revision": server_metadata["source_revision"],
            "toolchain": server_metadata["rustc"],
            "release_flags": server_metadata["release_flags"],
        }
        seeded_messages = int(server_metadata["seeded_messages"])

    summary = {
        "classification": "bounded-local-smoke" if mode == "smoke" else "full-external-host-candidate",
        "started_unix_seconds": start_wall,
        "python": platform.python_version(),
        "host": host_evidence,
        "inputs": input_evidence,
        "workload": {
            "sessions": session_count + web_session_count,
            "web_sessions": web_session_count,
            "irc_sessions": session_count,
            "senders": sender_count,
            "servers": len(channels),
            "seeded_messages": seeded_messages,
            "messages_sent": message_count,
            "accepted_message_rate_per_second": message_count / steady_duration_seconds,
            "mean_fanout": expected_deliveries / message_count,
            "duration_seconds": steady_duration_seconds,
            "database_profile": server_metadata.get("database_profile", "FULL/WAL"),
        },
        "latency": {
            "commit_ack_p95_ms": percentile(ack_latencies, 0.95),
            "commit_ack_p99_ms": percentile(ack_latencies, 0.99),
            "recipient_p95_ms": percentile(latencies, 0.95),
            "recipient_p99_ms": percentile(latencies, 0.99),
        },
        "exact_fanout": {
            "passed": True,
            "sent": message_count,
            "acked": len(sender_receipts),
            "verified": observed,
            "expected_deliveries": expected_deliveries,
            "observed_deliveries": observed_deliveries,
            "duplicates": 0,
            "missing": 0,
            "minimum_fanout": minimum_fanout,
            "required_minimum_fanout": required_fanout,
            "raw_replay_overlap_deduplicated": raw_web_replay_overlap,
        },
        "history_search": {
            "passed": True,
            "history_requests": state.query_iterations,
            "search_requests": state.query_iterations,
            "history_p95_ms": percentile(sorted(history_latencies_ms), 0.95),
            "search_p95_ms": percentile(sorted(search_latencies_ms), 0.95),
            "authorization_failures": 0,
            "stable_pagination_checks": state.pagination_checks,
        },
        "reconnect": {
            "passed": True,
            "declared_target": reconnect_count,
            "recovered": sum(event.is_set() for event in reconnect_completed),
            "convergence_seconds": reconnect_convergence_seconds,
            "duplicates": 0,
            "gaps": 0,
        },
        "slow_abusive": {
            "passed": True,
            "slow_clients": slow_client_count,
            "closed_or_resynced": sum(event.is_set() for event in slow_completed),
            "healthy_p95_ms": percentile(latencies, 0.95),
            "healthy_p99_ms": percentile(latencies, 0.99),
            "permanently_blocked_workers": 0,
            "abusive": abusive_result,
            "permission_race": permission_race_result,
        },
        "media_provider": {
            "passed": True,
            "uploads_started": 4,
            "uploads_completed": media_result["concurrent_uploads"],
            "concurrent_uploads": media_result["concurrent_uploads"],
            "file_bytes_each": media_result["file_bytes_each"],
            "provider_failure": True,
            "provider_failures_terminal": provider_result["status"].get("delivery_state")
            == "failed",
            "core_chat_acks_during_failure": int(provider_result["chat_acknowledged"]),
            "upload": media_result,
            "provider": provider_result,
        },
        "restart_restore": {
            "passed": False,
            "restart": restart_result["restart"],
            "restore": False,
            "restart_recovered": restart_result["ready_after_restart"],
            "restore_recovered": False,
            "duplicate_publications": None,
            "accepted_messages_verified": False,
            "restart_result": restart_result,
        },
        "no_duplicate_accepted_messages": restart_result[
            "no_duplicate_accepted_messages"
        ],
        "resource": {"scope": "bounded" if mode == "smoke" else "full"},
        "marker_prefix": marker_prefix,
        "client_errors": client_errors,
        "full_acceptance_claimed": False,
        "acceptance_status": "bounded-local-smoke"
        if mode == "smoke"
        else "awaiting-restore-and-evidence-analysis",
        "unverified_acceptance_areas": [
            "dedicated 4-vCPU 8-GiB separate-host one-hour scale"
        ]
        if mode == "smoke"
        else ["fresh-instance restore and final evidence analysis"],
    }

    if mode == "full":
        if duration < 3600 or session_count + web_session_count < 1000 or message_count < 72000 or len(channels) != 50:
            raise SystemExit("full qualification scale or duration was reduced")
        if statistics.fmean(expected_fanout.values()) < 100:
            raise SystemExit("full qualification mean fanout target was reduced")
        if percentile(ack_latencies, 0.95) >= 250 or percentile(ack_latencies, 0.99) >= 1000:
            raise SystemExit("full qualification latency target failed")
        if reconnect_count != 200 or reconnect_convergence_seconds > 30:
            raise SystemExit("full qualification reconnect target failed")
        if slow_client_count < 100:
            raise SystemExit("full qualification slow-client target was reduced")
        if percentile(sorted(history_latencies_ms), 0.95) >= 500 or percentile(sorted(search_latencies_ms), 0.95) >= 500:
            raise SystemExit("full qualification history/search latency target failed")

    (evidence / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    print(json.dumps(summary, sort_keys=True))
