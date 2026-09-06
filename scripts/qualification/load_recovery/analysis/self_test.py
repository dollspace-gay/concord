"""Self test for load/recovery evidence."""

from __future__ import annotations
import copy
from pathlib import Path
import tempfile
from typing import Any, Callable
from .validation import AnalysisError
from .analyze import analyze
from .fixtures import valid_samples, valid_summary, write_fixture
from .constants import GIB, MIB

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
