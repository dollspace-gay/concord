"""Shared configuration and worker state for one generator process.

Mutable collections and events are shared by identity. Access counters through
this module, and retain the existing locks around cross-thread mutations.
"""
from __future__ import annotations
import json
import os
import math
import socket
import ssl
import threading
import uuid
from pathlib import Path

host = os.environ["CONCORD_QUAL_IRC_HOST"]

port = int(os.environ["CONCORD_QUAL_IRC_PORT"])

token = os.environ.get("CONCORD_QUAL_IRC_TOKEN", "")

origin = os.environ["CONCORD_QUAL_HTTP_ORIGIN"].rstrip("/")

mode = os.environ["CONCORD_QUALIFICATION_MODE"]

session_count = int(os.environ["CONCORD_QUAL_SESSIONS"])

sender_count = int(os.environ["CONCORD_QUAL_SENDERS"])

message_count = int(os.environ["CONCORD_QUAL_MESSAGES"])

duration = int(os.environ["CONCORD_QUAL_DURATION_SECONDS"])

evidence = Path(os.environ["CONCORD_QUAL_EVIDENCE_DIR"])

samples = evidence / "time-series.jsonl"

telemetry_url = os.environ.get("CONCORD_QUAL_SERVER_TELEMETRY_URL")

metrics_session = os.environ["CONCORD_QUAL_METRICS_SESSION"]

control_command = os.environ.get("CONCORD_QUAL_CONTROL_COMMAND")

control_request = os.environ.get("CONCORD_QUAL_CONTROL_REQUEST")

control_response = os.environ.get("CONCORD_QUAL_CONTROL_RESPONSE")

marker_prefix = f"load-{uuid.uuid4()}-"

channels = ["#browser-fixture/general"]

if os.environ.get("CONCORD_QUAL_CHANNELS_FILE"):
    channels = json.load(open(os.environ["CONCORD_QUAL_CHANNELS_FILE"], encoding="utf-8"))

lock = threading.Lock()

sample_lock = threading.Lock()

receipts: dict[int, set[int]] = {}

first_receipt: dict[int, float] = {}

started: dict[int, float] = {}

errors: list[str] = []

stop = threading.Event()

query_stop = threading.Event()

reconnect_requested = threading.Event()

reconnect_release = threading.Event()

stall_requested = threading.Event()

stall_release = threading.Event()

source_ips = [value for value in os.environ.get("CONCORD_QUAL_SOURCE_IPS", "").split(",") if value]

tls_ca_file = os.environ.get("CONCORD_QUAL_IRC_CA_FILE")

tls_server_name = os.environ.get("CONCORD_QUAL_IRC_TLS_SERVER_NAME")

http_ca_file = os.environ.get("CONCORD_QUAL_HTTP_CA_FILE")

http_ssl_context = ssl.create_default_context(cafile=http_ca_file) if http_ca_file else None

telemetry_token = os.environ.get("CONCORD_QUAL_TELEMETRY_TOKEN")

tokens = [token] * session_count

if os.environ.get("CONCORD_QUAL_IRC_TOKENS_FILE"):
    tokens = json.load(open(os.environ["CONCORD_QUAL_IRC_TOKENS_FILE"], encoding="utf-8"))

registered = [threading.Event() for _ in range(session_count)]

web_inventory = []

if os.environ.get("CONCORD_QUAL_WEB_SESSIONS_FILE"):
    web_inventory = json.load(
        open(os.environ["CONCORD_QUAL_WEB_SESSIONS_FILE"], encoding="utf-8")
    )

web_session_count = int(os.environ.get("CONCORD_QUAL_WEB_SESSIONS", "0"))

reconnect_count = min(
    web_session_count,
    int(os.environ.get("CONCORD_QUAL_RECONNECT_SESSIONS", "200" if mode == "full" else str(web_session_count))),
)

slow_client_count = min(
    web_session_count,
    int(
        os.environ.get(
            "CONCORD_QUAL_SLOW_CLIENTS",
            str(math.ceil((session_count + web_session_count) * 0.1)),
        )
    ),
)

query_plan = []

if os.environ.get("CONCORD_QUAL_QUERY_PLAN"):
    query_plan = json.load(open(os.environ["CONCORD_QUAL_QUERY_PLAN"], encoding="utf-8"))

permission_race_plan = None

if os.environ.get("CONCORD_QUAL_PERMISSION_RACE_PLAN"):
    permission_race_plan = json.load(
        open(os.environ["CONCORD_QUAL_PERMISSION_RACE_PLAN"], encoding="utf-8")
    )

if web_session_count:
    if not isinstance(web_inventory, list) or len(web_inventory) < web_session_count:
        raise SystemExit(
            f"web session inventory must contain at least {web_session_count} entries"
        )
    web_inventory = web_inventory[:web_session_count]
    for item in web_inventory:
        if (
            not isinstance(item, dict)
            or not isinstance(item.get("cookie"), str)
            or not item["cookie"]
            or not isinstance(item.get("subscriptions"), list)
            or not isinstance(item.get("channels"), list)
            or not isinstance(item.get("server_id"), str)
            or not item["server_id"]
        ):
            raise SystemExit("web session inventory entry is malformed")

receiver_channels = [
    tuple(channels[(index + offset * max(1, len(channels) // 5)) % len(channels)] for offset in range(min(5, len(channels))))
    for index in range(session_count)
]

if sender_count > session_count:
    raise SystemExit("logical sender count cannot exceed the IRC session count")

sender_for_channel = {
    channel: next(
        (
            index
            for index in range(sender_count)
            if channel in receiver_channels[index]
        ),
        None,
    )
    for channel in channels
}

uncovered_sender_channels = [
    channel for channel, sender_index in sender_for_channel.items() if sender_index is None
]

if uncovered_sender_channels:
    raise SystemExit(
        "logical sender subset does not cover every channel: "
        + ", ".join(uncovered_sender_channels)
    )

expected_fanout = {channel: sum(channel in assigned for assigned in receiver_channels) for channel in channels}

expected_recipients = {
    channel: {index for index, assigned in enumerate(receiver_channels) if channel in assigned}
    for channel in channels
}

for web_offset, item in enumerate(web_inventory):
    global_index = session_count + web_offset
    for channel in item["channels"]:
        if channel not in expected_recipients:
            raise SystemExit(f"web session inventory references unknown channel {channel}")
        expected_recipients[channel].add(global_index)
        expected_fanout[channel] += 1

sequence_channels: dict[int, str] = {}

sequence_senders: dict[int, int] = {}

sender_receipts: dict[int, set[int]] = {}

sender_first_receipt: dict[int, float] = {}

web_logical_receipts: dict[int, set[int]] = {}

web_raw_receipts: dict[tuple[int, int], int] = {}

reconnect_disconnected = [threading.Event() for _ in range(reconnect_count)]

reconnect_completed = [threading.Event() for _ in range(reconnect_count)]

reconnect_finished_at = [0.0 for _ in range(reconnect_count)]

slow_stalled = [threading.Event() for _ in range(slow_client_count)]

slow_completed = [threading.Event() for _ in range(slow_client_count)]

slow_finished_at = [0.0 for _ in range(slow_client_count)]

query_latencies_ms: list[float] = []

history_latencies_ms: list[float] = []

search_latencies_ms: list[float] = []

query_iterations = 0

pagination_checks = 0

query_failures: list[str] = []

receiver_sockets: list[socket.socket | None] = [None] * session_count

web_registered = [threading.Event() for _ in range(web_session_count)]

web_cursors: dict[int, str] = {}
