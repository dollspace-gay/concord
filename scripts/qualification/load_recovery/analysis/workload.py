"""Workload for load/recovery evidence."""

from __future__ import annotations
from typing import Any, Callable
from .validation import AnalysisError, require_bool, require_integer, require_number, require_object, require_sha256, require_string
from .constants import GIB

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
