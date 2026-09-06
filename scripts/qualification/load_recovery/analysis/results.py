"""Results for load/recovery evidence."""

from __future__ import annotations
import math
from typing import Any, Callable
from .validation import AnalysisError, require_bool, require_integer, require_number, require_object, require_passed
from .constants import MIB

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
