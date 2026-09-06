#!/usr/bin/env python3
"""Validate Concord load/recovery evidence without exposing fixture secrets."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import sys
import tempfile
from typing import Any, Callable


GIB = 1024 * 1024 * 1024
MIB = 1024 * 1024
ACTIVE_COUNTS = ("connections", "jobs", "uploads")
SELF_TEST_QUERY_PLAN = b'[{"expected_total": 50, "query": "fixture"}]\n'


class AnalysisError(ValueError):
    """Evidence is absent, malformed, or does not meet the requested gate."""


def require_object(value: Any, context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise AnalysisError(f"{context} must be a JSON object")
    return value


def require_bool(value: Any, context: str) -> bool:
    if not isinstance(value, bool):
        raise AnalysisError(f"{context} must be a boolean")
    return value


def require_integer(value: Any, context: str, *, minimum: int = 0) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        raise AnalysisError(f"{context} must be an integer >= {minimum}")
    return value


def require_number(value: Any, context: str, *, minimum: float = 0.0) -> float:
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
        or value < minimum
    ):
        raise AnalysisError(f"{context} must be a finite number >= {minimum}")
    return float(value)


def require_string(value: Any, context: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise AnalysisError(f"{context} must be a non-empty string")
    return value


def require_sha256(value: Any, context: str) -> str:
    value = require_string(value, context)
    if len(value) != 64 or any(character not in "0123456789abcdef" for character in value):
        raise AnalysisError(f"{context} must be a lowercase SHA-256 digest")
    return value


def require_passed(summary: dict[str, Any], name: str) -> dict[str, Any]:
    result = require_object(summary.get(name), f"summary.{name}")
    if not require_bool(result.get("passed"), f"summary.{name}.passed"):
        raise AnalysisError(f"summary.{name}.passed must be true")
    return result


def load_summary(path: Path) -> dict[str, Any]:
    try:
        raw = path.read_text(encoding="utf-8")
    except OSError as error:
        raise AnalysisError(f"cannot read {path}: {error}") from error
    try:
        return require_object(json.loads(raw), "summary.json")
    except json.JSONDecodeError as error:
        raise AnalysisError(f"summary.json is invalid JSON: {error}") from error


def load_json(path: Path, context: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise AnalysisError(f"cannot read {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise AnalysisError(f"{context} is invalid JSON: {error}") from error


def load_samples(path: Path) -> list[dict[str, Any]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise AnalysisError(f"cannot read {path}: {error}") from error
    if not lines:
        raise AnalysisError("time-series.jsonl must contain at least one sample")
    samples: list[dict[str, Any]] = []
    for line_number, line in enumerate(lines, start=1):
        if not line.strip():
            raise AnalysisError(f"time-series.jsonl line {line_number} is blank")
        try:
            sample = json.loads(line)
        except json.JSONDecodeError as error:
            raise AnalysisError(
                f"time-series.jsonl line {line_number} is invalid JSON: {error}"
            ) from error
        samples.append(require_object(sample, f"time-series.jsonl line {line_number}"))
    return samples


def validate_context(summary: dict[str, Any], mode: str) -> dict[str, Any]:
    host = require_object(summary.get("host"), "summary.host")
    inputs = require_object(summary.get("inputs"), "summary.inputs")
    server_sha256 = require_sha256(
        inputs.get("server_sha256"), "summary.inputs.server_sha256"
    )
    dataset_sha256 = require_sha256(
        inputs.get("dataset_sha256"), "summary.inputs.dataset_sha256"
    )
    source_revision = require_string(
        inputs.get("source_revision"), "summary.inputs.source_revision"
    )
    target_hostname = require_string(
        host.get("server_hostname"), "summary.host.server_hostname"
    )
    generator_hostname = require_string(
        host.get("generator_hostname"), "summary.host.generator_hostname"
    )
    for field in ("server_platform", "generator_platform", "filesystem", "storage"):
        require_string(host.get(field), f"summary.host.{field}")
    for field in ("generator_sha256", "seed_sha256", "config_sha256", "query_mix_sha256"):
        require_sha256(inputs.get(field), f"summary.inputs.{field}")
    for field in ("toolchain", "release_flags"):
        require_string(inputs.get(field), f"summary.inputs.{field}")
    cpu_count = require_integer(host.get("cpu_count"), "summary.host.cpu_count", minimum=1)
    memory_bytes = require_integer(
        host.get("memory_bytes"), "summary.host.memory_bytes", minimum=1
    )
    separate = require_bool(
        host.get("separate_load_generator"), "summary.host.separate_load_generator"
    )
    if separate and target_hostname == generator_hostname:
        raise AnalysisError("target and generator hostnames must differ")
    if mode == "full":
        if cpu_count != 4 or not 7.5 * GIB <= memory_bytes <= 9 * GIB:
            raise AnalysisError(
                "full evidence requires the declared 4-vCPU, 8-GiB target "
                "(7.5-9 GiB OS-reporting tolerance)"
            )
        if not separate or target_hostname == generator_hostname:
            raise AnalysisError("full evidence requires separate target and generator hosts")

    return {
        "server_sha256": server_sha256,
        "dataset_sha256": dataset_sha256,
        "source_revision": source_revision,
        "target_hostname": target_hostname,
        "generator_hostname": generator_hostname,
        "cpu_count": cpu_count,
        "memory_bytes": memory_bytes,
        "separate_load_generator": separate,
    }


def validate_workload(summary: dict[str, Any], mode: str) -> dict[str, Any]:
    workload = require_object(summary.get("workload"), "summary.workload")
    values = {
        "irc_sessions": require_integer(
            workload.get("irc_sessions"), "summary.workload.irc_sessions", minimum=1
        ),
        "web_sessions": require_integer(
            workload.get("web_sessions"), "summary.workload.web_sessions", minimum=1
        ),
        "sessions": require_integer(
            workload.get("sessions"),
            "summary.workload.sessions",
            minimum=1,
        ),
        "servers": require_integer(
            workload.get("servers"), "summary.workload.servers", minimum=1
        ),
        "senders": require_integer(
            workload.get("senders"), "summary.workload.senders", minimum=1
        ),
        "seeded_messages": require_integer(
            workload.get("seeded_messages"),
            "summary.workload.seeded_messages",
            minimum=1,
        ),
        "messages_sent": require_integer(
            workload.get("messages_sent"), "summary.workload.messages_sent", minimum=1
        ),
        "duration_seconds": require_number(
            workload.get("duration_seconds"),
            "summary.workload.duration_seconds",
            minimum=0.001,
        ),
        "accepted_message_rate_per_second": require_number(
            workload.get("accepted_message_rate_per_second"),
            "summary.workload.accepted_message_rate_per_second",
            minimum=0.001,
        ),
        "mean_fanout": require_number(
            workload.get("mean_fanout"),
            "summary.workload.mean_fanout",
            minimum=1,
        ),
    }
    database_profile = require_string(
        workload.get("database_profile"), "summary.workload.database_profile"
    )
    if database_profile != "FULL/WAL":
        raise AnalysisError("summary.workload.database_profile must equal 'FULL/WAL'")
    values["database_profile"] = database_profile
    if values["sessions"] != values["irc_sessions"] + values["web_sessions"]:
        raise AnalysisError(
            "summary.workload.sessions must equal IRC plus web sessions"
        )
    if mode == "full":
        exact = {
            "irc_sessions": 800,
            "web_sessions": 200,
            "sessions": 1000,
            "servers": 50,
        }
        for field, expected in exact.items():
            if values[field] != expected:
                raise AnalysisError(f"full summary.workload.{field} must equal {expected}")
        if values["seeded_messages"] < 1_000_000:
            raise AnalysisError("full workload must seed at least 1,000,000 messages")
        if values["messages_sent"] < 72_000:
            raise AnalysisError("full workload must send at least 72,000 messages")
        if values["duration_seconds"] < 3_600:
            raise AnalysisError(
                "full workload must sustain steady chat for at least 3,600 seconds"
            )
        if values["accepted_message_rate_per_second"] < 20:
            raise AnalysisError("full workload must measure at least 20 accepted messages/second")
        if values["mean_fanout"] < 100:
            raise AnalysisError("full workload mean fanout must be at least 100")
    return values


def validate_results(
    summary: dict[str, Any], mode: str, workload: dict[str, Any]
) -> dict[str, Any]:
    latency = require_object(summary.get("latency"), "summary.latency")
    p95 = require_number(
        latency.get("commit_ack_p95_ms"), "summary.latency.commit_ack_p95_ms"
    )
    p99 = require_number(
        latency.get("commit_ack_p99_ms"), "summary.latency.commit_ack_p99_ms"
    )
    if p95 >= 250 or p99 >= 1_000:
        raise AnalysisError("commit-to-ack latency must have p95 <250 ms and p99 <1000 ms")

    fanout = require_passed(summary, "exact_fanout")
    fanout_values = {
        field: require_integer(fanout.get(field), f"summary.exact_fanout.{field}")
        for field in (
            "sent",
            "acked",
            "verified",
            "expected_deliveries",
            "observed_deliveries",
            "duplicates",
            "missing",
            "minimum_fanout",
            "required_minimum_fanout",
            "raw_replay_overlap_deduplicated",
        )
    }
    sent = workload["messages_sent"]
    if not (
        fanout_values["sent"]
        == fanout_values["acked"]
        == fanout_values["verified"]
        == sent
    ):
        raise AnalysisError("exact fanout message sent/acknowledged/verified totals must agree")
    if (
        fanout_values["expected_deliveries"] <= 0
        or fanout_values["observed_deliveries"] != fanout_values["expected_deliveries"]
        or any(fanout_values[field] for field in ("duplicates", "missing"))
        or fanout_values["minimum_fanout"] < fanout_values["required_minimum_fanout"]
    ):
        raise AnalysisError("exact fanout delivery totals contain a gap, duplicate, or surprise")
    recomputed_rate = sent / workload["duration_seconds"]
    if not math.isclose(
        workload["accepted_message_rate_per_second"],
        recomputed_rate,
        rel_tol=1e-6,
        abs_tol=1e-6,
    ):
        raise AnalysisError("accepted message rate is inconsistent with sent count and duration")
    recomputed_mean_fanout = fanout_values["expected_deliveries"] / sent
    if not math.isclose(
        workload["mean_fanout"],
        recomputed_mean_fanout,
        rel_tol=1e-9,
        abs_tol=1e-9,
    ):
        raise AnalysisError("mean fanout is inconsistent with exact delivery totals")

    history_search = require_passed(summary, "history_search")
    history_requests = require_integer(
        history_search.get("history_requests"),
        "summary.history_search.history_requests",
        minimum=1,
    )
    search_requests = require_integer(
        history_search.get("search_requests"),
        "summary.history_search.search_requests",
        minimum=1,
    )
    history_p95 = require_number(
        history_search.get("history_p95_ms"), "summary.history_search.history_p95_ms"
    )
    search_p95 = require_number(
        history_search.get("search_p95_ms"), "summary.history_search.search_p95_ms"
    )
    authorization_failures = require_integer(
        history_search.get("authorization_failures"),
        "summary.history_search.authorization_failures",
    )
    pagination_checks = require_integer(
        history_search.get("stable_pagination_checks"),
        "summary.history_search.stable_pagination_checks",
        minimum=1,
    )
    if history_requests != search_requests or pagination_checks != history_requests:
        raise AnalysisError("history/search and stable pagination measured totals must agree")
    if history_p95 >= 500 or search_p95 >= 500:
        raise AnalysisError("history/search p95 must be less than 500 ms")
    if authorization_failures:
        raise AnalysisError("history/search reported authorization failures")

    reconnect = require_passed(summary, "reconnect")
    declared_target = require_integer(
        reconnect.get("declared_target"), "summary.reconnect.declared_target", minimum=1
    )
    recovered = require_integer(reconnect.get("recovered"), "summary.reconnect.recovered")
    convergence = require_number(
        reconnect.get("convergence_seconds"), "summary.reconnect.convergence_seconds"
    )
    reconnect_duplicates = require_integer(
        reconnect.get("duplicates"), "summary.reconnect.duplicates"
    )
    reconnect_gaps = require_integer(reconnect.get("gaps"), "summary.reconnect.gaps")
    if recovered != declared_target or convergence > 30 or reconnect_duplicates or reconnect_gaps:
        raise AnalysisError("reconnect recovery target, deadline, or correctness failed")
    if mode == "full" and declared_target != 200:
        raise AnalysisError("full reconnect declared target must equal 200")

    slow = require_passed(summary, "slow_abusive")
    slow_clients = require_integer(
        slow.get("slow_clients"), "summary.slow_abusive.slow_clients", minimum=1
    )
    closed_or_resynced = require_integer(
        slow.get("closed_or_resynced"),
        "summary.slow_abusive.closed_or_resynced",
    )
    healthy_p95 = require_number(
        slow.get("healthy_p95_ms"), "summary.slow_abusive.healthy_p95_ms"
    )
    healthy_p99 = require_number(
        slow.get("healthy_p99_ms"), "summary.slow_abusive.healthy_p99_ms"
    )
    blocked_workers = require_integer(
        slow.get("permanently_blocked_workers"),
        "summary.slow_abusive.permanently_blocked_workers",
    )
    if closed_or_resynced < slow_clients or healthy_p95 >= 250 or healthy_p99 >= 1_000:
        raise AnalysisError("slow/abusive cleanup or healthy-client latency failed")
    if blocked_workers:
        raise AnalysisError("slow/abusive workload left permanently blocked workers")
    if mode == "full" and slow_clients < 100:
        raise AnalysisError("full slow/abusive workload requires at least 100 slow clients")
    abusive = require_object(slow.get("abusive"), "summary.slow_abusive.abusive")
    require_number(
        abusive.get("fragmented_command_latency_ms"),
        "summary.slow_abusive.abusive.fragmented_command_latency_ms",
    )
    if abusive.get("invalid_command_code") != "INVALID_INPUT":
        raise AnalysisError("invalid abusive command was not rejected as INVALID_INPUT")
    if abusive.get("oversized_command_code") not in ("INVALID_INPUT", "CONNECTION_CLOSED"):
        raise AnalysisError("oversized abusive command was not rejected")
    if abusive.get("rate_limit_code") != "RATE_LIMITED" or abusive.get(
        "connection_recovered_after_rate_limit"
    ) is not True:
        raise AnalysisError("abusive rate-limit and recovery evidence is incomplete")
    permission = require_object(
        slow.get("permission_race"), "summary.slow_abusive.permission_race"
    )
    require_integer(
        permission.get("concurrent_probe_count"),
        "summary.slow_abusive.permission_race.concurrent_probe_count",
        minimum=1,
    )
    statuses = permission.get("concurrent_statuses")
    if (
        not isinstance(statuses, list)
        or not statuses
        or any(
            isinstance(status, bool) or not isinstance(status, int) or status >= 500
            for status in statuses
        )
        or permission.get("post_revocation_status") not in (403, 404)
    ):
        raise AnalysisError("permission-race evidence is absent or unsafe")

    media = require_passed(summary, "media_provider")
    file_bytes = require_integer(
        media.get("file_bytes_each"), "summary.media_provider.file_bytes_each", minimum=1
    )
    concurrent_uploads = require_integer(
        media.get("concurrent_uploads"), "summary.media_provider.concurrent_uploads"
    )
    uploads_completed = require_integer(
        media.get("uploads_completed"), "summary.media_provider.uploads_completed"
    )
    provider_failure = require_bool(
        media.get("provider_failure"), "summary.media_provider.provider_failure"
    )
    provider_terminal = require_bool(
        media.get("provider_failures_terminal"),
        "summary.media_provider.provider_failures_terminal",
    )
    core_acks = require_integer(
        media.get("core_chat_acks_during_failure"),
        "summary.media_provider.core_chat_acks_during_failure",
        minimum=1,
    )
    if concurrent_uploads != 4 or uploads_completed != 4:
        raise AnalysisError(
            "media/provider workload must complete exactly four concurrent uploads"
        )
    if not provider_failure:
        raise AnalysisError("media/provider workload must observe a provider failure")
    if mode == "full" and not provider_terminal:
        raise AnalysisError("full media/provider workload requires a terminal provider failure")
    if mode == "full" and file_bytes != 100 * MIB:
        raise AnalysisError("full media workload files must each equal 100 MiB")
    upload = require_object(media.get("upload"), "summary.media_provider.upload")
    if (
        upload.get("all_four_overlapped") is not True
        or upload.get("chat_acknowledged_while_four_admitted") is not True
        or upload.get("retained_cursor_present") is not True
        or require_integer(
            upload.get("attachments_cleaned_up"),
            "summary.media_provider.upload.attachments_cleaned_up",
        )
        != 4
        or require_integer(
            upload.get("concurrent_uploads"),
            "summary.media_provider.upload.concurrent_uploads",
        )
        != 4
        or require_integer(
            upload.get("file_bytes_each"),
            "summary.media_provider.upload.file_bytes_each",
            minimum=1,
        )
        != file_bytes
    ):
        raise AnalysisError("media upload overlap, size, or cleanup evidence failed")
    require_number(
        upload.get("maximum_duration_seconds"),
        "summary.media_provider.upload.maximum_duration_seconds",
        minimum=0.001,
    )
    require_number(
        upload.get("chat_ack_latency_ms"),
        "summary.media_provider.upload.chat_ack_latency_ms",
    )
    provider = require_object(media.get("provider"), "summary.media_provider.provider")
    if provider.get("chat_acknowledged") is not True or provider.get(
        "retained_cursor_present"
    ) is not True:
        raise AnalysisError("provider-failure chat acknowledgement evidence failed")
    provider_status = require_object(
        provider.get("status"), "summary.media_provider.provider.status"
    )
    if (
        provider_status.get("delivery_error") != "webhook_transport_unavailable"
        or provider_status.get("job_error") != "webhook_transport_unavailable"
        or provider_status.get("last_status") is not None
    ):
        raise AnalysisError("provider failure status is inconsistent or unsafe")
    if mode == "full" and (
        provider.get("terminal_required") is not True
        or provider_status.get("delivery_state") != "failed"
        or provider_status.get("job_state") != "failed"
        or provider_status.get("delivery_attempts") != 8
        or provider_status.get("job_attempts") != 8
    ):
        raise AnalysisError("full provider failure did not reach its bounded terminal state")
    if mode != "full" and (
        provider.get("terminal_required") is not False
        or provider_terminal
        or provider_status.get("delivery_state") != "pending"
        or provider_status.get("job_state") != "pending"
        or not isinstance(provider_status.get("delivery_attempts"), int)
        or provider_status["delivery_attempts"] < 1
        or not isinstance(provider_status.get("job_attempts"), int)
        or provider_status["job_attempts"] < 1
    ):
        raise AnalysisError("smoke provider failure lacks a classified bounded pending attempt")

    restart_restore = require_passed(summary, "restart_restore")
    for field in ("restart", "restore", "restart_recovered", "restore_recovered"):
        if not require_bool(
            restart_restore.get(field), f"summary.restart_restore.{field}"
        ):
            raise AnalysisError(f"summary.restart_restore.{field} must be true")
    restored_verified = require_bool(
        restart_restore.get("accepted_messages_verified"),
        "summary.restart_restore.accepted_messages_verified",
    )
    if not restored_verified:
        raise AnalysisError("summary.restart_restore.accepted_messages_verified must be true")
    restore_result = require_object(
        restart_restore.get("restore_result"), "summary.restart_restore.restore_result"
    )
    restored_main_messages = require_integer(
        restore_result.get("restored_main_messages"),
        "summary.restart_restore.restore_result.restored_main_messages",
    )
    if restored_main_messages != sent:
        raise AnalysisError("restored main-message total must equal exact fanout sent total")
    for field in (
        "restored_pending_jobs_reconciled",
        "restored_provider_messages",
        "restored_media_stress_messages",
        "restored_restart_messages",
    ):
        restored_count = require_integer(
            restore_result.get(field),
            f"summary.restart_restore.restore_result.{field}",
        )
        if restored_count != 1:
            raise AnalysisError(
                f"summary.restart_restore.restore_result.{field} must equal 1"
            )
    if not require_bool(
        restore_result.get("old_session_invalidated"),
        "summary.restart_restore.restore_result.old_session_invalidated",
    ):
        raise AnalysisError(
            "summary.restart_restore.restore_result.old_session_invalidated must be true"
        )
    duplicate_publications = require_integer(
        restart_restore.get("duplicate_publications"),
        "summary.restart_restore.duplicate_publications",
    )
    if duplicate_publications:
        raise AnalysisError("restart/restore observed duplicate external publication")
    restart_result = require_object(
        restart_restore.get("restart_result"), "summary.restart_restore.restart_result"
    )
    if (
        restart_result.get("ready_after_restart") is not True
        or restart_result.get("logical_message_count") != 1
        or restart_result.get("no_duplicate_accepted_messages") is not True
        or restart_result.get("retained_cursor_resumed") is not True
    ):
        raise AnalysisError("restart result lacks ready, cursor, or exact-once evidence")
    if (
        restore_result.get("integrity") != "ok"
        or restore_result.get("foreign_key_violations") != 0
        or restore_result.get("missing_media") != 0
        or restore_result.get("external_jobs_paused") is not True
        or restore_result.get("duplicate_publications") != 0
        or restore_result.get("accepted_messages_verified") is not True
        or not isinstance(restore_result.get("declared_restore_point"), str)
        or not restore_result["declared_restore_point"].strip()
    ):
        raise AnalysisError("restore result lacks coherent, paused, exact-once evidence")

    if not require_bool(
        summary.get("no_duplicate_accepted_messages"),
        "summary.no_duplicate_accepted_messages",
    ):
        raise AnalysisError("summary.no_duplicate_accepted_messages must be true")

    return {
        "commit_ack_p95_ms": p95,
        "commit_ack_p99_ms": p99,
        "messages_sent": sent,
        "messages_verified": fanout_values["verified"],
        "recipient_deliveries_verified": fanout_values["observed_deliveries"],
        "history_requests": history_requests,
        "search_requests": search_requests,
        "history_p95_ms": history_p95,
        "search_p95_ms": search_p95,
        "reconnect_declared_target": declared_target,
        "reconnect_recovered": recovered,
        "reconnect_convergence_seconds": convergence,
        "slow_clients": slow_clients,
        "closed_or_resynced": closed_or_resynced,
        "media_file_bytes_each": file_bytes,
        "concurrent_uploads": concurrent_uploads,
        "provider_failure_terminal": provider_terminal,
        "core_chat_acks_during_failure": core_acks,
        "restart_restore_messages_verified": restored_main_messages,
        "no_duplicate_accepted_messages": True,
    }


def parse_telemetry(
    samples: list[dict[str, Any]], mode: str
) -> list[dict[str, Any]]:
    present = ["server_telemetry" in sample for sample in samples]
    if not all(present):
        raise AnalysisError(f"{mode} evidence requires server_telemetry in every sample")

    telemetry: list[dict[str, Any]] = []
    count_fields_present: list[bool] = []
    previous_timestamp: float | None = None
    phase_order = {"warmup": 0, "steady": 1, "stress": 2, "post_disconnect": 3}
    previous_phase = -1
    for index, sample in enumerate(samples, start=1):
        if "sample_errors" in sample:
            raise AnalysisError(
                f"time-series.jsonl sample {index} reports required observability errors"
            )
        for field in ("ready_status", "concord_metrics_status"):
            status = require_integer(
                sample.get(field), f"time-series.jsonl sample {index}.{field}"
            )
            if status != 200:
                raise AnalysisError(
                    f"time-series.jsonl sample {index}.{field} must equal 200"
                )
        timestamp = require_number(
            sample.get("unix_seconds"),
            f"time-series.jsonl sample {index}.unix_seconds",
        )
        if previous_timestamp is not None and timestamp <= previous_timestamp:
            raise AnalysisError("telemetry unix_seconds values must be strictly increasing")
        previous_timestamp = timestamp
        phase = require_string(
            sample.get("phase"), f"time-series.jsonl sample {index}.phase"
        )
        if phase not in phase_order:
            raise AnalysisError(
                "telemetry phase must be 'warmup', 'steady', 'stress', or 'post_disconnect'"
            )
        if phase_order[phase] < previous_phase:
            raise AnalysisError("telemetry phases must remain in lifecycle order")
        previous_phase = phase_order[phase]
        row = require_object(
            sample["server_telemetry"],
            f"time-series.jsonl sample {index}.server_telemetry",
        )
        rss = require_integer(
            row.get("rss_bytes"),
            f"time-series.jsonl sample {index}.server_telemetry.rss_bytes",
            minimum=1,
        )
        parsed: dict[str, Any] = {
            "unix_seconds": timestamp,
            "phase": phase,
            "rss_bytes": rss,
        }
        fields = [field in row for field in ACTIVE_COUNTS]
        if any(fields) and not all(fields):
            raise AnalysisError(
                f"time-series.jsonl sample {index} must provide all active counts or none"
            )
        count_fields_present.append(all(fields))
        if all(fields):
            for field in ACTIVE_COUNTS:
                parsed[field] = require_integer(
                    row[field],
                    f"time-series.jsonl sample {index}.server_telemetry.{field}",
                )
        telemetry.append(parsed)

    if any(count_fields_present) and not all(count_fields_present):
        raise AnalysisError("active counts must be present in every telemetry sample or none")
    if not all(count_fields_present):
        raise AnalysisError(
            "server telemetry requires connections, jobs, and uploads counts in every sample"
        )
    return telemetry


def nondecreasing_tail_growth(rss_values: list[int]) -> tuple[int, int]:
    start = len(rss_values) - 1
    while start > 0 and rss_values[start - 1] <= rss_values[start]:
        start -= 1
    return len(rss_values) - start, rss_values[-1] - rss_values[start]


def analyze_resources(
    telemetry: list[dict[str, Any]],
    mode: str,
    declared_scope: str,
    expected_sessions: int,
    reconnect_target: int,
) -> dict[str, Any]:
    expected_scope = "full" if mode == "full" else "bounded"
    if declared_scope != expected_scope:
        raise AnalysisError(
            f"summary.resource.scope must be {expected_scope!r} in {mode} mode"
        )
    if not telemetry:
        return {"scope": declared_scope, "telemetry_supplied": False}

    phases = {
        phase: [row for row in telemetry if row["phase"] == phase]
        for phase in ("warmup", "steady", "stress", "post_disconnect")
    }
    minimum_samples = (
        {"warmup": 5, "steady": 61, "stress": 4, "post_disconnect": 5}
        if mode == "full"
        else {"warmup": 1, "steady": 1, "stress": 1, "post_disconnect": 1}
    )
    for phase, minimum in minimum_samples.items():
        if len(phases[phase]) < minimum:
            raise AnalysisError(
                f"{mode} telemetry requires at least {minimum} {phase} samples"
            )
    if mode == "full":
        steady_times = [row["unix_seconds"] for row in phases["steady"]]
        steady_span = steady_times[-1] - steady_times[0]
        if steady_span < 3_600:
            raise AnalysisError("full steady telemetry must span at least 3,600 seconds")
        maximum_gap = max(
            later - earlier for earlier, later in zip(steady_times, steady_times[1:])
        )
        if maximum_gap > 60.5:
            raise AnalysisError("full steady telemetry sampling gap exceeds 60.5 seconds")
        steady_connections = [row["connections"] for row in phases["steady"]]
        minimum_during_reconnect = expected_sessions - reconnect_target
        if min(steady_connections) < minimum_during_reconnect:
            raise AnalysisError(
                "full steady telemetry fell below the declared reconnect occupancy floor"
            )
        full_occupancy_samples = sum(
            connections >= expected_sessions for connections in steady_connections
        )
        if full_occupancy_samples / len(steady_connections) < 0.9:
            raise AnalysisError(
                "full steady telemetry did not sustain declared session occupancy "
                "for at least 90% of samples"
            )
    else:
        steady_span = (
            phases["steady"][-1]["unix_seconds"]
            - phases["steady"][0]["unix_seconds"]
        )
        maximum_gap = (
            max(
                later["unix_seconds"] - earlier["unix_seconds"]
                for earlier, later in zip(phases["steady"], phases["steady"][1:])
            )
            if len(phases["steady"]) > 1
            else 0.0
        )

    if max(row["uploads"] for row in phases["stress"]) < 4:
        raise AnalysisError(
            f"{mode} stress telemetry never measured four concurrent staging uploads"
        )

    rss_values = [row["rss_bytes"] for row in telemetry]
    baseline = statistics.median(
        [row["rss_bytes"] for row in phases["warmup"][:5]]
    )
    high_water = max(rss_values)
    post_disconnect = statistics.median(
        [row["rss_bytes"] for row in phases["post_disconnect"][-5:]]
    )
    retained_allowance = max(64 * MIB, 0.25 * (high_water - baseline))
    post_limit = baseline + retained_allowance
    tail_samples, tail_growth = nondecreasing_tail_growth(rss_values)

    if high_water >= 2 * GIB:
        raise AnalysisError("server RSS high-water must be less than 2 GiB")
    if post_disconnect > post_limit:
        raise AnalysisError("post-disconnect median RSS exceeds the reclamation limit")
    if tail_samples >= 8 and tail_growth > 64 * MIB:
        raise AnalysisError(
            "server RSS has a nondecreasing tail of at least 8 samples with over 64 MiB growth"
        )

    final_counts: dict[str, int] | None = None
    if all(field in telemetry[-1] for field in ACTIVE_COUNTS):
        final_counts = {field: telemetry[-1][field] for field in ACTIVE_COUNTS}
        if any(final_counts.values()):
            raise AnalysisError("final server active connections/jobs/uploads must all be zero")

    return {
        "scope": declared_scope,
        "telemetry_supplied": True,
        "sample_count": len(telemetry),
        "phase_samples": {phase: len(rows) for phase, rows in phases.items()},
        "steady_span_seconds": steady_span,
        "maximum_steady_sample_gap_seconds": maximum_gap,
        "baseline_rss_bytes": baseline,
        "high_water_rss_bytes": high_water,
        "post_disconnect_rss_bytes": post_disconnect,
        "post_disconnect_limit_bytes": post_limit,
        "nondecreasing_tail_samples": tail_samples,
        "nondecreasing_tail_growth_bytes": tail_growth,
        "final_active_counts": final_counts,
        "full_occupancy_sample_ratio": (
            full_occupancy_samples / len(phases["steady"])
            if mode == "full"
            else None
        ),
    }


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            dir=path.parent,
            prefix=f".{path.name}.",
            suffix=".tmp",
            delete=False,
        ) as handle:
            temporary = Path(handle.name)
            json.dump(value, handle, indent=2, sort_keys=True, allow_nan=False)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        temporary = None
        directory_fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def validate_supporting_artifacts(
    evidence: Path,
    summary: dict[str, Any],
    mode: str,
    workload: dict[str, Any],
    checks: dict[str, Any],
) -> dict[str, Any]:
    if mode != "full":
        return {"required": False}

    host = require_object(summary.get("host"), "summary.host")
    inputs = require_object(summary.get("inputs"), "summary.inputs")
    metadata = require_object(
        load_json(evidence / "server-metadata.json", "server-metadata.json"),
        "server-metadata.json",
    )
    field_pairs = (
        (host, "server_hostname", metadata, "hostname"),
        (host, "server_platform", metadata, "kernel"),
        (host, "filesystem", metadata, "filesystem"),
        (host, "storage", metadata, "storage"),
        (inputs, "server_sha256", metadata, "server_sha256"),
        (inputs, "seed_sha256", metadata, "seed_sha256"),
        (inputs, "dataset_sha256", metadata, "dataset_sha256"),
        (inputs, "config_sha256", metadata, "config_sha256"),
        (inputs, "query_mix_sha256", metadata, "query_mix_sha256"),
        (inputs, "source_revision", metadata, "source_revision"),
        (inputs, "toolchain", metadata, "rustc"),
        (inputs, "release_flags", metadata, "release_flags"),
    )
    for left, left_key, right, right_key in field_pairs:
        if left.get(left_key) != right.get(right_key):
            raise AnalysisError(
                f"summary {left_key} does not match server-metadata.json {right_key}"
            )
    for key in ("cpu_count", "memory_bytes"):
        metadata_value = require_integer(
            metadata.get(key), f"server-metadata.json.{key}", minimum=1
        )
        if metadata_value != host[key]:
            raise AnalysisError(f"summary host {key} does not match server-metadata.json")
    if metadata.get("database_profile") != workload["database_profile"]:
        raise AnalysisError("summary database profile does not match server-metadata.json")
    if require_integer(
        metadata.get("seeded_messages"),
        "server-metadata.json.seeded_messages",
        minimum=1,
    ) != workload["seeded_messages"]:
        raise AnalysisError("summary seeded-message count does not match server-metadata.json")
    if require_integer(
        metadata.get("configured_max_upload_bytes"),
        "server-metadata.json.configured_max_upload_bytes",
        minimum=1,
    ) != checks["media_file_bytes_each"]:
        raise AnalysisError("summary media size does not match server-metadata.json")

    query_plan_path = evidence / "query-plan.json"
    query_plan_bytes = query_plan_path.read_bytes() if query_plan_path.is_file() else None
    if query_plan_bytes is None:
        raise AnalysisError(f"cannot read {query_plan_path}: file is missing")
    if hashlib.sha256(query_plan_bytes).hexdigest() != inputs["query_mix_sha256"]:
        raise AnalysisError("query-plan.json SHA-256 does not match summary inputs")
    query_plan = load_json(query_plan_path, "query-plan.json")
    if not isinstance(query_plan, list) or not query_plan:
        raise AnalysisError("query-plan.json must be a non-empty array")

    channels = load_json(evidence / "channel-inventory.json", "channel-inventory.json")
    if (
        not isinstance(channels, list)
        or len(channels) != 50
        or len(set(channels)) != 50
        or any(not isinstance(channel, str) or not channel.startswith("#") for channel in channels)
    ):
        raise AnalysisError("channel-inventory.json must contain 50 unique channel aliases")

    credentials = require_object(
        load_json(
            evidence / "credential-inventory.redacted.json",
            "credential-inventory.redacted.json",
        ),
        "credential-inventory.redacted.json",
    )
    credential_count = require_integer(
        credentials.get("credential_count"),
        "credential-inventory.redacted.json.credential_count",
        minimum=1,
    )
    if credential_count < workload["irc_sessions"] + workload["senders"]:
        raise AnalysisError("redacted credential inventory is smaller than the IRC workload")

    generator_path = Path(__file__).with_name("load-recovery-generator.py")
    try:
        generator_sha256 = hashlib.sha256(generator_path.read_bytes()).hexdigest()
    except OSError as error:
        raise AnalysisError(f"cannot hash {generator_path}: {error}") from error
    if generator_sha256 != inputs["generator_sha256"]:
        raise AnalysisError("load generator SHA-256 does not match summary inputs")

    return {
        "required": True,
        "server_metadata_matched": True,
        "query_plan_sha256_matched": True,
        "channel_count": len(channels),
        "credential_count": credential_count,
        "generator_sha256_matched": True,
    }


def analyze(mode: str, evidence: Path) -> dict[str, Any]:
    if not evidence.is_dir():
        raise AnalysisError(f"evidence directory does not exist: {evidence}")
    summary = load_summary(evidence / "summary.json")
    samples = load_samples(evidence / "time-series.jsonl")
    context = validate_context(summary, mode)
    workload = validate_workload(summary, mode)
    checks = validate_results(summary, mode, workload)

    claimed = require_bool(
        summary.get("full_acceptance_claimed"), "summary.full_acceptance_claimed"
    )
    if claimed != (mode == "full"):
        raise AnalysisError(
            f"summary.full_acceptance_claimed must be {str(mode == 'full').lower()} in {mode} mode"
        )
    expected_classification = (
        "full-external-host-candidate" if mode == "full" else "bounded-local-smoke"
    )
    if summary.get("classification") != expected_classification:
        raise AnalysisError(
            f"summary.classification must equal {expected_classification!r} in {mode} mode"
        )
    expected_status = "full-acceptance-candidate" if mode == "full" else "bounded-local-smoke"
    if summary.get("acceptance_status") != expected_status:
        raise AnalysisError(
            f"summary.acceptance_status must equal {expected_status!r} in {mode} mode"
        )
    client_errors = summary.get("client_errors")
    if not isinstance(client_errors, list) or client_errors:
        raise AnalysisError("summary.client_errors must be an empty array")
    unverified = summary.get("unverified_acceptance_areas")
    if not isinstance(unverified, list):
        raise AnalysisError("summary.unverified_acceptance_areas must be an array")
    if mode == "full" and unverified:
        raise AnalysisError("full evidence cannot retain unverified acceptance areas")
    resource = require_object(summary.get("resource"), "summary.resource")
    scope = resource.get("scope")
    if not isinstance(scope, str):
        raise AnalysisError("summary.resource.scope must be a string")

    telemetry = parse_telemetry(samples, mode)
    resource_analysis = analyze_resources(
        telemetry,
        mode,
        scope,
        workload["sessions"],
        checks["reconnect_declared_target"],
    )
    if mode == "full" and not math.isclose(
        resource_analysis["steady_span_seconds"],
        workload["duration_seconds"],
        rel_tol=0.0,
        abs_tol=resource_analysis["maximum_steady_sample_gap_seconds"],
    ):
        raise AnalysisError(
            "declared workload duration does not agree with measured steady telemetry span"
        )
    supporting_artifacts = validate_supporting_artifacts(
        evidence, summary, mode, workload, checks
    )
    result = {
        "schema_version": 1,
        "mode": mode,
        "classification": "full-acceptance" if mode == "full" else "bounded-local-smoke",
        "full_acceptance_claimed": claimed,
        "time_series_samples": len(samples),
        "context": context,
        "workload": workload,
        "result_checks": checks,
        "resource": resource_analysis,
        "supporting_artifacts": supporting_artifacts,
    }
    atomic_write_json(evidence / "analysis.json", result)
    return result


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
            "generator_sha256": hashlib.sha256(
                Path(__file__).with_name("load-recovery-generator.py").read_bytes()
            ).hexdigest(),
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


def run_self_test() -> int:
    failures_tested = 0
    with tempfile.TemporaryDirectory(prefix="concord-load-analysis-") as temporary:
        root = Path(temporary)

        full_pass = root / "full-pass"
        write_fixture(full_pass, valid_summary(), valid_samples())
        full_result = analyze("full", full_pass)
        assert full_result["classification"] == "full-acceptance"
        assert (full_pass / "analysis.json").is_file()

        smoke_pass = root / "smoke-pass"
        write_fixture(
            smoke_pass,
            valid_summary(mode="smoke"),
            valid_samples(),
        )
        smoke_result = analyze("smoke", smoke_pass)
        assert smoke_result["classification"] == "bounded-local-smoke"
        assert smoke_result["resource"]["telemetry_supplied"]

        def expect_failure(
            name: str,
            summary_change: Callable[[dict[str, Any]], None] | None = None,
            sample_change: Callable[[list[dict[str, Any]]], None] | None = None,
            fixture_change: Callable[[Path], None] | None = None,
            *,
            mode: str = "full",
            expected_error: str | None = None,
        ) -> None:
            nonlocal failures_tested
            summary = copy.deepcopy(valid_summary(mode=mode))
            samples = copy.deepcopy(valid_samples())
            if summary_change is not None:
                summary_change(summary)
            if sample_change is not None:
                sample_change(samples)
            case = root / name
            write_fixture(case, summary, samples)
            if fixture_change is not None:
                fixture_change(case)
            try:
                analyze(mode, case)
            except AnalysisError as error:
                if expected_error is not None and expected_error not in str(error):
                    raise AssertionError(
                        f"self-test case {name} failed for the wrong reason: {error}"
                    ) from error
                failures_tested += 1
                return
            raise AssertionError(f"self-test case unexpectedly passed: {name}")

        expect_failure(
            "fanout-failure", lambda value: value["exact_fanout"].update(passed=False)
        )
        expect_failure(
            "fanout-mean-not-recomputed",
            lambda value: value["workload"].update(mean_fanout=101),
            expected_error="mean fanout is inconsistent",
        )
        expect_failure(
            "message-rate-not-recomputed",
            lambda value: value["workload"].update(
                accepted_message_rate_per_second=21
            ),
            expected_error="message rate is inconsistent",
        )

        def mismatched_duration(summary: dict[str, Any]) -> None:
            messages = 73_220
            summary["workload"]["duration_seconds"] = 3_661.0
            summary["workload"]["messages_sent"] = messages
            summary["workload"]["accepted_message_rate_per_second"] = 20.0
            summary["exact_fanout"].update(
                sent=messages,
                acked=messages,
                verified=messages,
                expected_deliveries=messages * 100,
                observed_deliveries=messages * 100,
            )
            summary["restart_restore"]["restore_result"][
                "restored_main_messages"
            ] = messages

        expect_failure(
            "duration-telemetry-mismatch",
            summary_change=mismatched_duration,
            expected_error="does not agree with measured steady telemetry span",
        )
        expect_failure(
            "vacuous-target-one",
            lambda value: value["workload"].update(
                irc_sessions=1, web_sessions=1, sessions=2
            ),
            expected_error="full summary.workload.irc_sessions must equal 800",
        )
        expect_failure(
            "vacuous-one-sample",
            sample_change=lambda value: value.__setitem__(slice(None), value[:1]),
            expected_error="at least 5 warmup samples",
        )
        expect_failure(
            "oversized-host",
            lambda value: value["host"].update(memory_bytes=1024 * GIB),
            expected_error="7.5-9 GiB",
        )
        expect_failure(
            "missing-server-metadata",
            fixture_change=lambda value: (value / "server-metadata.json").unlink(),
            expected_error="server-metadata.json",
        )
        expect_failure(
            "query-plan-hash-mismatch",
            fixture_change=lambda value: (value / "query-plan.json").write_text(
                "[]\n", encoding="utf-8"
            ),
            expected_error="SHA-256",
        )

        def shrink_credentials(root_path: Path) -> None:
            (root_path / "credential-inventory.redacted.json").write_text(
                '{"credential_count": 1}\n', encoding="utf-8"
            )

        expect_failure(
            "credential-inventory-too-small",
            fixture_change=shrink_credentials,
            expected_error="smaller than the IRC workload",
        )
        expect_failure(
            "reconnect-mismatch",
            lambda value: value["reconnect"].update(recovered=199),
        )
        expect_failure(
            "upload-count", lambda value: value["media_provider"].update(concurrent_uploads=3)
        )
        expect_failure(
            "provider-failure-missing",
            lambda value: value["media_provider"].update(provider_failure=False),
        )
        expect_failure(
            "provider-not-terminal",
            lambda value: value["media_provider"]["provider"]["status"].update(
                delivery_attempts=7
            ),
            expected_error="bounded terminal state",
        )
        expect_failure(
            "restart-missing", lambda value: value["restart_restore"].update(restart=False)
        )
        expect_failure(
            "restore-missing", lambda value: value["restart_restore"].update(restore=False)
        )
        expect_failure(
            "restore-total-mismatch",
            lambda value: value["restart_restore"]["restore_result"].update(
                restored_main_messages=1
            ),
            expected_error="restored main-message total",
        )
        for field in (
            "restored_pending_jobs_reconciled",
            "restored_provider_messages",
            "restored_media_stress_messages",
            "restored_restart_messages",
        ):
            expect_failure(
                f"{field}-mismatch",
                lambda value, field=field: value["restart_restore"][
                    "restore_result"
                ].update({field: 0}),
                expected_error=f"{field} must equal 1",
            )
        expect_failure(
            "old-session-retained",
            lambda value: value["restart_restore"]["restore_result"].update(
                old_session_invalidated=False
            ),
            expected_error="old_session_invalidated must be true",
        )
        expect_failure(
            "duplicate-accepted",
            lambda value: value.update(no_duplicate_accepted_messages=False),
        )
        expect_failure(
            "claim-mismatch", lambda value: value.update(full_acceptance_claimed=False)
        )
        expect_failure(
            "scope-mismatch", lambda value: value["resource"].update(scope="bounded")
        )
        expect_failure(
            "missing-telemetry", sample_change=lambda value: value[0].pop("server_telemetry")
        )
        expect_failure(
            "smoke-missing-telemetry",
            sample_change=lambda value: [sample.pop("server_telemetry") for sample in value],
            mode="smoke",
            expected_error="smoke evidence requires server_telemetry in every sample",
        )
        expect_failure(
            "telemetry-sample-error",
            sample_change=lambda value: value[0].update(
                sample_errors={"server_telemetry": "unavailable"}
            ),
            expected_error="reports required observability errors",
        )
        expect_failure(
            "metrics-status-forbidden",
            sample_change=lambda value: value[0].update(concord_metrics_status=403),
            expected_error="concord_metrics_status must equal 200",
        )
        expect_failure(
            "ready-status-failure",
            sample_change=lambda value: value[0].update(ready_status=503),
            expected_error="ready_status must equal 200",
        )
        expect_failure(
            "generic-sample-error",
            sample_change=lambda value: value[0].update(
                sample_errors={"concord_metrics": "timed out"}
            ),
            expected_error="reports required observability errors",
        )
        expect_failure(
            "invalid-rss",
            sample_change=lambda value: value[0]["server_telemetry"].update(rss_bytes=True),
        )
        expect_failure(
            "partial-counts",
            sample_change=lambda value: value[0]["server_telemetry"].pop("uploads"),
        )
        expect_failure(
            "rss-high-water",
            sample_change=lambda value: value[5]["server_telemetry"].update(rss_bytes=2 * GIB),
        )
        expect_failure(
            "active-final",
            sample_change=lambda value: value[-1]["server_telemetry"].update(connections=1),
        )
        expect_failure(
            "missing-stress-phase",
            sample_change=lambda value: value.__setitem__(
                slice(None), [sample for sample in value if sample["phase"] != "stress"]
            ),
            expected_error="at least 4 stress samples",
        )

        def no_four_upload_peak(samples: list[dict[str, Any]]) -> None:
            for sample in samples:
                if sample["phase"] == "stress":
                    sample["server_telemetry"]["uploads"] = 3

        expect_failure(
            "missing-upload-stress-peak",
            sample_change=no_four_upload_peak,
            expected_error="four concurrent staging uploads",
        )

        def mostly_reconnecting(samples: list[dict[str, Any]]) -> None:
            steady = [sample for sample in samples if sample["phase"] == "steady"]
            for sample in steady[:7]:
                sample["server_telemetry"]["connections"] = 800

        expect_failure(
            "steady-occupancy-not-sustained",
            sample_change=mostly_reconnecting,
            expected_error="at least 90%",
        )

        def sparse_steady(samples: list[dict[str, Any]]) -> None:
            steady_seen = 0
            for sample in samples:
                if sample["phase"] == "steady":
                    if steady_seen >= 30:
                        sample["unix_seconds"] += 1
                    steady_seen += 1
                elif sample["phase"] in ("stress", "post_disconnect"):
                    sample["unix_seconds"] += 1

        expect_failure(
            "steady-sampling-gap",
            sample_change=sparse_steady,
            expected_error="sampling gap exceeds 60.5 seconds",
        )
        expect_failure(
            "phase-order-regression",
            sample_change=lambda value: value[10].update(phase="warmup"),
            expected_error="lifecycle order",
        )

        def unreclaimed(samples: list[dict[str, Any]]) -> None:
            for sample in samples[-5:]:
                sample["server_telemetry"]["rss_bytes"] = 300 * MIB

        expect_failure("rss-unreclaimed", sample_change=unreclaimed)

        def monotonic_tail(samples: list[dict[str, Any]]) -> None:
            for index, sample in enumerate(samples[-8:]):
                sample["server_telemetry"]["rss_bytes"] = (100 + index * 10) * MIB

        expect_failure(
            "rss-monotonic-tail",
            sample_change=monotonic_tail,
            expected_error="nondecreasing tail",
        )

        invalid_json = root / "invalid-json"
        write_fixture(invalid_json, valid_summary(), valid_samples())
        (invalid_json / "time-series.jsonl").write_text("{invalid\n", encoding="utf-8")
        try:
            analyze("full", invalid_json)
        except AnalysisError:
            failures_tested += 1
        else:
            raise AssertionError("invalid JSON self-test unexpectedly passed")

    print(
        f"PASS analyze-load-recovery-evidence self-test failures_tested={failures_tested}"
    )
    return 0


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


if __name__ == "__main__":
    raise SystemExit(main())
