"""Resources for load/recovery evidence."""

from __future__ import annotations
import statistics
from typing import Any, Callable
from .validation import AnalysisError, require_integer, require_number, require_object, require_string
from .constants import ACTIVE_COUNTS, GIB, MIB

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
