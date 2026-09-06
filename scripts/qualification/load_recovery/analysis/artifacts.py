"""Artifacts for load/recovery evidence."""

from __future__ import annotations
from ..provenance import generator_fingerprint
import hashlib
import json
import os
from pathlib import Path
import tempfile
from typing import Any, Callable
from .validation import AnalysisError, load_json, require_integer, require_object

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

    generator_path = Path(__file__).resolve().parents[3] / "load-recovery-generator.py"
    try:
        generator_sha256 = generator_fingerprint()
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
