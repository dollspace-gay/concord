"""Http for load/recovery qualification."""
from __future__ import annotations
import json
import subprocess
import time
import urllib.request
import urllib.error
from .state import control_command, http_ssl_context, origin

def authenticated_json_request(
    cookie: str, path: str, *, method: str = "GET", body: bytes | None = None
) -> tuple[int, object, float]:
    request = urllib.request.Request(
        origin + path,
        data=body,
        method=method,
        headers={
            "Cookie": "concord_session=" + cookie,
            "Accept": "application/json",
        },
    )
    started_at = time.monotonic()
    try:
        with urllib.request.urlopen(
            request, timeout=10, context=http_ssl_context
        ) as response:
            payload = response.read()
            status = response.status
    except urllib.error.HTTPError as error:
        payload = error.read()
        status = error.code
    elapsed_ms = (time.monotonic() - started_at) * 1000
    try:
        decoded = json.loads(payload) if payload else None
    except json.JSONDecodeError:
        decoded = payload.decode(errors="replace")
    return status, decoded, elapsed_ms

def control_action(action: str, *arguments: str, timeout_seconds: float = 30) -> dict:
    if not control_command:
        raise RuntimeError("qualification control command is unavailable")
    completed = subprocess.run(
        [control_command, action, *arguments],
        check=False,
        capture_output=True,
        text=True,
        timeout=timeout_seconds,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"qualification control action {action} failed: {completed.stderr.strip()}"
        )
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"qualification control action {action} returned invalid JSON"
        ) from error
    if not isinstance(result, dict):
        raise RuntimeError(f"qualification control action {action} returned no object")
    return result
