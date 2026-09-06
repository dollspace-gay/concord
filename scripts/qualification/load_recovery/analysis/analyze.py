"""Analyze for load/recovery evidence."""

from __future__ import annotations
import math
from pathlib import Path
from typing import Any, Callable
from .validation import AnalysisError, load_samples, load_summary, require_bool, require_object
from .workload import validate_context, validate_workload
from .results import validate_results
from .resources import analyze_resources, parse_telemetry
from .artifacts import atomic_write_json, validate_supporting_artifacts

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
