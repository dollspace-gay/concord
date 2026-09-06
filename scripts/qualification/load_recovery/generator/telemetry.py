"""Telemetry for load/recovery qualification."""
from __future__ import annotations
import json
import time
import urllib.request
import urllib.error
from .state import http_ssl_context, metrics_session, origin, sample_lock, samples, telemetry_token, telemetry_url

def fetch_server_telemetry() -> dict:
    if not telemetry_url:
        raise RuntimeError("target telemetry is unavailable")
    telemetry_request = urllib.request.Request(
        telemetry_url,
        headers=(
            {"Authorization": "Bearer " + telemetry_token}
            if telemetry_token
            else {}
        ),
    )
    with urllib.request.urlopen(
        telemetry_request, timeout=5, context=http_ssl_context
    ) as response:
        telemetry = json.loads(response.read())
    if not isinstance(telemetry, dict):
        raise RuntimeError("target telemetry did not return a JSON object")
    return telemetry

def collect_sample(phase: str) -> None:
    with sample_lock:
        sample = {
            "unix_seconds": time.time(),
            "phase": phase,
            "generator_rss_kib": __import__("resource").getrusage(0).ru_maxrss,
        }
        sample_errors = {}
        try:
            with urllib.request.urlopen(
                origin + "/health/ready", timeout=5, context=http_ssl_context
            ) as response:
                sample["ready_status"] = response.status
        except Exception as exc:
            sample_errors["ready"] = str(exc)
        if telemetry_url:
            try:
                sample["server_telemetry"] = fetch_server_telemetry()
            except Exception as exc:
                sample_errors["server_telemetry"] = str(exc)
        try:
            metrics_request = urllib.request.Request(
                origin + "/metrics",
                headers={"Cookie": "concord_session=" + metrics_session},
            )
            with urllib.request.urlopen(
                metrics_request, timeout=5, context=http_ssl_context
            ) as response:
                sample["concord_metrics_status"] = response.status
                sample["concord_metrics"] = response.read().decode(errors="replace")
        except urllib.error.HTTPError as exc:
            sample["concord_metrics_status"] = exc.code
        except Exception as exc:
            sample_errors["concord_metrics"] = str(exc)
        if sample_errors:
            sample["sample_errors"] = sample_errors
        with samples.open("a", encoding="utf-8") as output:
            output.write(json.dumps(sample, sort_keys=True) + "\n")
