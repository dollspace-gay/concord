"""Orchestrate clients, steady workload, stress probes, and evidence collection."""
from __future__ import annotations
import os
import threading
import time
from .state import channels, duration, errors, expected_recipients, lock, marker_prefix, message_count, mode, permission_race_plan, query_failures, query_plan, query_stop, receipts, receiver_sockets, reconnect_completed, reconnect_count, reconnect_finished_at, reconnect_release, reconnect_requested, registered, sender_for_channel, sender_receipts, sequence_channels, sequence_senders, session_count, slow_client_count, slow_completed, stall_release, stall_requested, started, stop, web_inventory, web_registered, web_session_count
from . import state
from .irc_transport import send_line
from .telemetry import collect_sample
from .provider_probe import provider_failure_probe
from .queries import query_worker
from .abuse_probe import abusive_client_probe
from .permission_probe import permission_race_probe
from .media_probe import media_stress_probe
from .restart_probe import restart_deduplication_probe
from .irc_worker import client
from .web_worker import web_client
from .reporting import RunMeasurements, write_summary

def main() -> None:
    threads = [threading.Thread(target=client, args=(index,), daemon=True) for index in range(session_count)]

    for index, thread in enumerate(threads):
        thread.start()
        if not registered[index].wait(20):
            with lock:
                detail = "; ".join(errors)
            raise SystemExit(detail or f"client {index} did not register")

    with lock:
        if errors:
            raise SystemExit("; ".join(errors))

    web_threads = [
        threading.Thread(target=web_client, args=(index, item), daemon=True)
        for index, item in enumerate(web_inventory)
    ]

    for index, thread in enumerate(web_threads):
        thread.start()
        if not web_registered[index].wait(20):
            with lock:
                detail = "; ".join(errors)
            raise SystemExit(detail or f"web client {index} did not synchronize")

    with lock:
        if errors:
            raise SystemExit("; ".join(errors))

    if not isinstance(query_plan, list) or not query_plan:
        raise SystemExit("qualification query plan must contain at least one entry")

    for item in query_plan:
        required_query_fields = {"session_index", "server_id", "channel", "query", "expected_total"}
        if not isinstance(item, dict) or required_query_fields - item.keys():
            raise SystemExit("qualification query plan entry is malformed")
        if not 0 <= int(item["session_index"]) < web_session_count:
            raise SystemExit("qualification query plan references an unavailable web session")

    query_worker_count = int(
        os.environ.get("CONCORD_QUAL_QUERY_WORKERS", "4" if mode == "full" else "2")
    )

    query_threads = [
        threading.Thread(target=query_worker, args=(index,), daemon=True)
        for index in range(query_worker_count)
    ]

    for thread in query_threads:
        thread.start()

    time.sleep(1)

    for _ in range(5):
        collect_sample("warmup")
        time.sleep(0.05 if mode == "smoke" else 1.0)

    start_wall = time.time()

    start_mono = time.monotonic()

    interval = duration / max(message_count, 1)

    reconnect_at = max(1, message_count // 4)

    reconnect_hold_seconds = float(os.environ.get("CONCORD_QUAL_RECONNECT_HOLD_SECONDS", "2"))

    slow_hold_seconds = float(os.environ.get("CONCORD_QUAL_SLOW_HOLD_SECONDS", "5"))

    if reconnect_hold_seconds <= 0 or reconnect_hold_seconds >= 30:
        raise SystemExit("reconnect interruption must be greater than zero and less than 30 seconds")

    if slow_hold_seconds <= 0 or slow_hold_seconds >= 30:
        raise SystemExit("slow-reader interruption must be greater than zero and less than 30 seconds")

    stall_at = max(reconnect_at + 1, message_count // 2)

    reconnect_started_at = None

    stall_started_at = None

    for sequence in range(message_count):
        target = start_mono + sequence * interval
        if target > time.monotonic():
            time.sleep(target - time.monotonic())
        started[sequence] = time.monotonic()
        destination = channels[sequence % len(channels)]
        sequence_channels[sequence] = destination
        sender_index = sender_for_channel[destination]
        if sender_index is None or receiver_sockets[sender_index] is None:
            raise SystemExit(f"logical sender is unavailable for {destination}")
        sequence_senders[sequence] = sender_index
        send_line(
            receiver_sockets[sender_index],
            f"PRIVMSG {destination} :{marker_prefix}{sequence}",
        )
        if sequence == reconnect_at and reconnect_count:
            reconnect_started_at = time.monotonic()
            reconnect_requested.set()
        if (
            reconnect_started_at is not None
            and not reconnect_release.is_set()
            and time.monotonic() - reconnect_started_at >= reconnect_hold_seconds
        ):
            reconnect_release.set()
        if sequence == stall_at and slow_client_count:
            stall_started_at = time.monotonic()
            stall_requested.set()
        if (
            stall_started_at is not None
            and not stall_release.is_set()
            and time.monotonic() - stall_started_at >= slow_hold_seconds
        ):
            stall_release.set()
        if sequence == 0 or sequence % max(1, message_count // max(duration, 1)) == 0:
            collect_sample("steady")

    steady_deadline = start_mono + duration

    if steady_deadline > time.monotonic():
        time.sleep(steady_deadline - time.monotonic())

    collect_sample("steady")

    steady_duration_seconds = float(duration)

    stall_release.set()

    reconnect_release.set()

    recovery_deadline = time.monotonic() + 30

    for event in reconnect_completed + slow_completed:
        remaining = recovery_deadline - time.monotonic()
        if remaining <= 0 or not event.wait(remaining):
            raise SystemExit("WebSocket reconnect/slow-client recovery exceeded 30 seconds")

    reconnect_convergence_seconds = (
        max(reconnect_finished_at) - reconnect_started_at
        if reconnect_started_at is not None and reconnect_finished_at
        else 0.0
    )

    wait_deadline = time.monotonic() + 30

    while time.monotonic() < wait_deadline:
        with lock:
            complete_fanout = all(
                receipts.get(sequence) == expected_recipients[sequence_channels[sequence]]
                for sequence in range(message_count)
            )
            if complete_fanout and len(sender_receipts) == message_count:
                break
        time.sleep(0.05)

    stress_sample_stop = threading.Event()

    def sample_stress() -> None:
        interval_seconds = 1.0 if mode == "full" else 0.1
        while not stress_sample_stop.is_set():
            collect_sample("stress")
            stress_sample_stop.wait(interval_seconds)

    stress_sample_thread = threading.Thread(target=sample_stress, daemon=True)

    stress_sample_thread.start()

    abusive_result = abusive_client_probe(web_inventory[0])

    provider_webhook_id = os.environ.get("CONCORD_QUAL_PROVIDER_WEBHOOK_ID")

    if not provider_webhook_id:
        raise SystemExit("qualification provider-failure webhook ID is required")

    provider_result = provider_failure_probe(web_inventory[0], provider_webhook_id)

    media_result = media_stress_probe()

    if not isinstance(permission_race_plan, dict):
        raise SystemExit("qualification permission-race plan is required")

    permission_race_result = permission_race_probe(permission_race_plan)

    stress_sample_stop.set()

    stress_sample_thread.join(timeout=10)

    if stress_sample_thread.is_alive():
        raise SystemExit("stress telemetry sampler did not stop")

    query_stop.set()

    for thread in query_threads:
        thread.join(timeout=15)

    if any(thread.is_alive() for thread in query_threads):
        raise SystemExit("history/search worker did not stop within its request deadline")

    with lock:
        if query_failures:
            raise SystemExit("; ".join(query_failures))
        if state.query_iterations < query_worker_count:
            raise SystemExit("history/search workload did not complete one iteration per worker")

    stop.set()

    for thread in threads:
        thread.join(timeout=3)

    for thread in web_threads:
        thread.join(timeout=3)

    if any(thread.is_alive() for thread in threads + web_threads):
        raise SystemExit("steady client thread did not stop before restart/resource reclamation")

    restart_result = restart_deduplication_probe(web_inventory[0])

    collect_sample("stress")

    for _ in range(5):
        collect_sample("post_disconnect")
        time.sleep(0.05 if mode == "smoke" else 1.0)

    write_summary(RunMeasurements(
        start_wall=start_wall,
        steady_duration_seconds=steady_duration_seconds,
        reconnect_convergence_seconds=reconnect_convergence_seconds,
        abusive_result=abusive_result,
        permission_race_result=permission_race_result,
        media_result=media_result,
        provider_result=provider_result,
        restart_result=restart_result,
    ))
