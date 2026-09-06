"""Validation for load/recovery evidence."""

from __future__ import annotations
import json
import math
from pathlib import Path
from typing import Any, Callable

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
