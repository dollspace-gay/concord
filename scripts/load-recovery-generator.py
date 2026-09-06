#!/usr/bin/env python3
"""Dependency-free IRC load/correctness generator for Concord qualification."""

import json
import os
import platform
import base64
import hashlib
import http.client
import math
import re
import socket
import ssl
import statistics
import struct
import subprocess
import threading
import time
import urllib.request
import urllib.error
import uuid
from pathlib import Path
from urllib.parse import quote, urlencode, urlparse

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


def source_address(index: int) -> tuple[str, int] | None:
    if not source_ips:
        return None
    # Concord admits at most five connections per source IP. Spread the
    # receiver and sender inventory across every supplied address accordingly.
    return (source_ips[index // 5], 0)


def send_line(sock: socket.socket, line: str) -> None:
    sock.sendall((line + "\r\n").encode())


def connect(index: int) -> socket.socket:
    raw = socket.create_connection((host, port), timeout=15, source_address=source_address(index))
    if not tls_ca_file:
        return raw
    context = ssl.create_default_context(cafile=tls_ca_file)
    try:
        return context.wrap_socket(raw, server_hostname=tls_server_name or host)
    except Exception:
        raw.close()
        raise


class WebSocketClient:
    def __init__(self, cookie: str, source_index: int):
        parsed = urlparse(origin)
        if parsed.scheme not in ("http", "https") or not parsed.hostname:
            raise RuntimeError("qualification HTTP origin is invalid")
        port_number = parsed.port or (443 if parsed.scheme == "https" else 80)
        raw = socket.create_connection(
            (parsed.hostname, port_number),
            timeout=15,
            source_address=source_address(source_index),
        )
        if parsed.scheme == "https":
            raw = (http_ssl_context or ssl.create_default_context()).wrap_socket(
                raw, server_hostname=parsed.hostname
            )
        raw.settimeout(5)
        key = base64.b64encode(os.urandom(16)).decode()
        host_header = parsed.hostname if parsed.port is None else f"{parsed.hostname}:{parsed.port}"
        request = (
            "GET /ws HTTP/1.1\r\n"
            f"Host: {host_header}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Origin: {origin}\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            f"Cookie: concord_session={cookie}\r\n\r\n"
        )
        raw.sendall(request.encode())
        response = b""
        while b"\r\n\r\n" not in response:
            response += raw.recv(4096)
            if len(response) > 16_384:
                raise RuntimeError("oversized WebSocket handshake")
        headers, self.buffer = response.split(b"\r\n\r\n", 1)
        lines = headers.decode(errors="replace").split("\r\n")
        if " 101 " not in lines[0]:
            raise RuntimeError(f"WebSocket handshake rejected: {lines[0]}")
        response_headers = {
            name.lower(): value.strip()
            for line in lines[1:]
            if ":" in line
            for name, value in [line.split(":", 1)]
        }
        expected_accept = base64.b64encode(
            hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()
        ).decode()
        if response_headers.get("sec-websocket-accept") != expected_accept:
            raise RuntimeError("WebSocket handshake accept key mismatch")
        self.socket = raw

    def send_frame(self, opcode: int, payload: bytes = b"") -> None:
        mask = os.urandom(4)
        length = len(payload)
        header = bytearray([0x80 | opcode])
        if length < 126:
            header.append(0x80 | length)
        elif length <= 0xFFFF:
            header.append(0x80 | 126)
            header.extend(struct.pack("!H", length))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack("!Q", length))
        header.extend(mask)
        header.extend(bytes(value ^ mask[index % 4] for index, value in enumerate(payload)))
        self.socket.sendall(header)

    def send_json(self, value: dict) -> None:
        self.send_frame(1, json.dumps(value, separators=(",", ":")).encode())

    def send_fragmented_json(self, value: dict, fragment_bytes: int = 3) -> None:
        payload = json.dumps(value, separators=(",", ":")).encode()
        fragments = [
            payload[offset : offset + fragment_bytes]
            for offset in range(0, len(payload), fragment_bytes)
        ]
        for index, fragment in enumerate(fragments):
            mask = os.urandom(4)
            opcode = 1 if index == 0 else 0
            final = index == len(fragments) - 1
            header = bytearray([(0x80 if final else 0) | opcode])
            length = len(fragment)
            if length < 126:
                header.append(0x80 | length)
            else:
                header.append(0x80 | 126)
                header.extend(struct.pack("!H", length))
            header.extend(mask)
            header.extend(
                bytes(value ^ mask[position % 4] for position, value in enumerate(fragment))
            )
            self.socket.sendall(header)

    def read_exact(self, length: int) -> bytes:
        while len(self.buffer) < length:
            chunk = self.socket.recv(max(4096, length - len(self.buffer)))
            if not chunk:
                raise EOFError("WebSocket peer closed")
            self.buffer += chunk
        value, self.buffer = self.buffer[:length], self.buffer[length:]
        return value

    def receive_json(self) -> dict | None:
        while True:
            first, second = self.read_exact(2)
            opcode = first & 0x0F
            length = second & 0x7F
            if length == 126:
                length = struct.unpack("!H", self.read_exact(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self.read_exact(8))[0]
            if second & 0x80:
                mask = self.read_exact(4)
            else:
                mask = None
            payload = self.read_exact(length)
            if mask:
                payload = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
            if opcode == 8:
                raise EOFError("WebSocket close frame")
            if opcode == 9:
                self.send_frame(10, payload)
                continue
            if opcode != 1:
                continue
            value = json.loads(payload)
            if isinstance(value, dict):
                return value

    def close(self) -> None:
        self.socket.close()


def marker_sequences(value: object) -> set[int]:
    """Return exact qualification sequence markers from a decoded event."""
    encoded = json.dumps(value, separators=(",", ":"))
    return {
        int(match.group(1))
        for match in re.finditer(re.escape(marker_prefix) + r"(\d+)(?!\d)", encoded)
    }


def record_web_event(web_index: int, global_index: int, event: dict) -> None:
    for sequence in marker_sequences(event):
        with lock:
            web_raw_receipts[(web_index, sequence)] = (
                web_raw_receipts.get((web_index, sequence), 0) + 1
            )
            web_logical_receipts.setdefault(web_index, set()).add(sequence)
            receipts.setdefault(sequence, set()).add(global_index)


def synchronize_websocket(
    client_socket: WebSocketClient,
    web_index: int,
    global_index: int,
    subscriptions: list[str],
    cursor: str | None,
) -> tuple[str, str]:
    """Synchronize through the complete bounded replay and return cursor/generation."""
    operation_generation = ""
    while True:
        request_id = f"qualification-sync-{web_index}-{uuid.uuid4()}"
        message = {
            "type": "sync",
            "request_id": request_id,
            "protocol_version": 2,
            "subscriptions": subscriptions,
            "limit": 100,
        }
        if cursor is not None:
            message["cursor"] = cursor
        client_socket.send_json(message)
        while True:
            event = client_socket.receive_json()
            if event is None:
                continue
            record_web_event(web_index, global_index, event)
            if event.get("request_id") != request_id:
                continue
            event_type = event.get("type")
            if event_type in ("command_error", "resync_required"):
                raise RuntimeError(f"WebSocket synchronization failed: {event}")
            if event_type not in ("sync_snapshot", "replay_batch"):
                continue
            projection = event.get("snapshot") or event.get("batch") or {}
            if not isinstance(projection.get("cursor"), str):
                raise RuntimeError("WebSocket synchronization returned no cursor")
            if not isinstance(projection.get("operation_generation"), str):
                raise RuntimeError("WebSocket synchronization returned no operation generation")
            cursor = projection["cursor"]
            operation_generation = projection["operation_generation"]
            has_more = bool(projection.get("has_more", False))
            break
        if not has_more:
            return cursor, operation_generation


def join_web_channels(client_socket: WebSocketClient, item: dict) -> None:
    for channel in item["channels"]:
        channel_name = "#" + channel.rsplit("/", 1)[-1].lstrip("#")
        client_socket.send_json(
            {
                "type": "join_channel",
                "server_id": item["server_id"],
                "channel": channel_name,
            }
        )


def open_websocket(web_index: int, item: dict) -> WebSocketClient:
    client_socket = WebSocketClient(
        item["cookie"], session_count + web_index
    )
    client_socket.socket.settimeout(0.25)
    return client_socket


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


def provider_failure_probe(item: dict, webhook_id: str) -> dict:
    armed = control_action("provider-arm", webhook_id)
    if armed.get("armed") is not True:
        raise RuntimeError("controlled provider-failure webhook was not armed")
    client_socket = open_websocket(0, item)
    try:
        cursor, operation_generation = synchronize_websocket(
            client_socket, 0, session_count, item["subscriptions"], None
        )
        join_web_channels(client_socket, item)
        request_id = f"provider-failure-{uuid.uuid4()}"
        content = f"{marker_prefix}provider-failure"
        started_at = time.monotonic()
        client_socket.send_json(
            {
                "type": "send_message",
                "operation_generation": operation_generation,
                "request_id": request_id,
                "client_message_id": request_id,
                "conversation_id": item["subscriptions"][0],
                "server_id": item["server_id"],
                "channel": "#" + item["channels"][0].rsplit("/", 1)[-1].lstrip("#"),
                "content": content,
                "content_format": "plain",
                "reply_to": None,
                "attachment_ids": None,
                "mentions": [],
                "nonce": request_id,
            }
        )
        acknowledgement = receive_until(
            client_socket,
            lambda event: event.get("type") in ("message_ack", "command_error")
            and event.get("request_id") == request_id,
        )
        ack_ms = (time.monotonic() - started_at) * 1000
        if acknowledgement.get("type") != "message_ack":
            raise RuntimeError(f"provider-failure chat send was rejected: {acknowledgement}")
        if acknowledgement.get("client_message_id") != request_id:
            raise RuntimeError("provider-failure chat acknowledgement lost correlation")
        terminal_required = mode == "full"
        deadline = time.monotonic() + (390 if terminal_required else 15)
        status = {}
        while time.monotonic() < deadline:
            status = control_action("provider-status", webhook_id)
            if status.get("found") and (
                status.get("delivery_state") == "failed"
                or (not terminal_required and int(status.get("delivery_attempts", 0)) >= 1)
            ):
                break
            time.sleep(1)
        if not status.get("found"):
            raise RuntimeError("provider-failure send created no webhook delivery")
        if status.get("delivery_error") != "webhook_transport_unavailable":
            raise RuntimeError(f"provider failure was misclassified: {status}")
        if status.get("last_status") is not None:
            raise RuntimeError("DNS provider failure incorrectly recorded an HTTP status")
        if status.get("job_error") != "webhook_transport_unavailable":
            raise RuntimeError("provider failure did not converge across job/delivery state")
        if terminal_required:
            if (
                status.get("delivery_state") != "failed"
                or status.get("job_state") != "failed"
                or int(status.get("delivery_attempts", 0)) != 8
                or int(status.get("job_attempts", 0)) != 8
            ):
                raise RuntimeError(f"provider failure did not reach bounded terminal state: {status}")
        result = {
            "chat_acknowledged": True,
            "chat_ack_latency_ms": ack_ms,
            "retained_cursor_present": bool(cursor),
            "terminal_required": terminal_required,
            "status": status,
        }
        disarmed = control_action("provider-disarm", webhook_id)
        if disarmed.get("disarmed") is not True:
            raise RuntimeError("controlled provider-failure webhook was not disarmed")
        result["disarmed_before_later_chat"] = True
        return result
    finally:
        client_socket.close()


def query_worker(worker_index: int) -> None:
    global pagination_checks, query_iterations
    try:
        while not query_stop.is_set():
            plan = query_plan[worker_index % len(query_plan)]
            inventory_index = int(plan["session_index"])
            item = web_inventory[inventory_index]
            cookie = item["cookie"]
            server_id = plan["server_id"]
            channel = plan["channel"]
            history_path = (
                f"/api/channels/{quote(channel, safe='')}/messages?"
                + urlencode({"server_id": server_id, "limit": 50})
            )
            status, history, history_ms = authenticated_json_request(cookie, history_path)
            if status != 200 or not isinstance(history, dict):
                raise RuntimeError(f"history request failed with status {status}")
            history_messages = history.get("messages")
            if not isinstance(history_messages, list):
                raise RuntimeError("history response omitted messages")
            history_ids = [message.get("id") for message in history_messages]
            if len(history_ids) != len(set(history_ids)):
                raise RuntimeError("history page contained duplicate message IDs")
            if len(history_ids) < int(plan.get("history_min_count", 1)):
                raise RuntimeError("history page was smaller than the declared fixture")

            search_parameters = {
                "server_id": server_id,
                "q": plan["query"],
                "channel": channel,
                "limit": int(plan.get("page_size", 25)),
            }
            search_path = "/api/search?" + urlencode(search_parameters)
            status, first_page, search_ms = authenticated_json_request(cookie, search_path)
            if status != 200 or not isinstance(first_page, dict):
                raise RuntimeError(f"search request failed with status {status}")
            expected_total = int(plan["expected_total"])
            if first_page.get("total_count") != expected_total:
                raise RuntimeError(
                    f"search total changed: expected={expected_total} actual={first_page.get('total_count')}"
                )
            if first_page.get("restarted", False):
                raise RuntimeError("fresh search unexpectedly reported restarted pagination")
            first_ids = [result.get("id") for result in first_page.get("results", [])]
            if len(first_ids) != len(set(first_ids)):
                raise RuntimeError("search page contained duplicate message IDs")
            continuation = first_page.get("next_continuation")
            pagination_ms = 0.0
            if continuation:
                second_parameters = dict(search_parameters)
                second_parameters["continuation"] = continuation
                status, second_page, pagination_ms = authenticated_json_request(
                    cookie, "/api/search?" + urlencode(second_parameters)
                )
                if status != 200 or not isinstance(second_page, dict):
                    raise RuntimeError(f"continued search failed with status {status}")
                if second_page.get("total_count") != expected_total:
                    raise RuntimeError("search total changed across continuation")
                second_ids = [
                    result.get("id") for result in second_page.get("results", [])
                ]
                if set(first_ids) & set(second_ids):
                    raise RuntimeError("search continuation repeated a logical message")
            with lock:
                query_latencies_ms.extend([history_ms, search_ms, pagination_ms])
                history_latencies_ms.append(history_ms)
                search_latencies_ms.append(search_ms)
                query_iterations += 1
                pagination_checks += int(bool(continuation))
            query_stop.wait(float(plan.get("interval_seconds", 2.0)))
    except Exception as exc:
        with lock:
            query_failures.append(f"query worker {worker_index}: {exc}")
        query_stop.set()


def receive_until(
    client_socket: WebSocketClient, predicate, deadline_seconds: float = 10.0
) -> dict:
    deadline = time.monotonic() + deadline_seconds
    while time.monotonic() < deadline:
        try:
            event = client_socket.receive_json()
        except TimeoutError:
            continue
        if event is not None and predicate(event):
            return event
    raise RuntimeError("timed out waiting for the expected WebSocket event")


def abusive_client_probe(item: dict) -> dict:
    client_socket = open_websocket(0, item)
    try:
        fragmented_started = time.monotonic()
        client_socket.send_fragmented_json({"type": "list_servers"})
        receive_until(client_socket, lambda event: event.get("type") == "server_list")
        fragmented_ms = (time.monotonic() - fragmented_started) * 1000

        invalid_request_id = f"invalid-{uuid.uuid4()}"
        client_socket.send_frame(
            1,
            json.dumps(
                {"type": "invalid_qualification_command", "request_id": invalid_request_id}
            ).encode(),
        )
        invalid = receive_until(
            client_socket,
            lambda event: event.get("request_id") == invalid_request_id,
        )
        if invalid.get("type") != "command_error" or invalid.get("code") != "INVALID_INPUT":
            raise RuntimeError(f"invalid command was not rejected precisely: {invalid}")

        oversized_request_id = f"oversized-{uuid.uuid4()}"
        client_socket.send_json(
            {
                "type": "send_message",
                "request_id": oversized_request_id,
                "client_message_id": oversized_request_id,
                "operation_generation": "qualification-invalid-generation",
                "conversation_id": item["subscriptions"][0],
                "server_id": item["server_id"],
                "channel": "#" + item["channels"][0].rsplit("/", 1)[-1].lstrip("#"),
                "content": "x" * 65_537,
                "content_format": "plain",
                "reply_to": None,
                "attachment_ids": None,
                "mentions": [],
                "nonce": oversized_request_id,
            }
        )
        try:
            oversized = receive_until(
                client_socket,
                lambda event: event.get("request_id") == oversized_request_id
                or (
                    event.get("type") == "command_committed"
                    and event.get("receipt", {}).get("request_id")
                    == oversized_request_id
                ),
            )
            if oversized.get("type") != "command_error":
                raise RuntimeError("oversized command reached canonical acceptance")
            oversized_code = oversized.get("code")
        except EOFError:
            oversized_code = "CONNECTION_CLOSED"
            client_socket.close()
            client_socket = open_websocket(0, item)

        rate_ids = [f"abuse-{uuid.uuid4()}-{index}" for index in range(140)]
        for request_id in rate_ids:
            client_socket.send_json(
                {
                    "type": "search_messages",
                    "request_id": request_id,
                    "server_id": item["server_id"],
                    "query": "historical",
                    "channel": "#" + item["channels"][0].rsplit("/", 1)[-1].lstrip("#"),
                    "limit": 1,
                    "offset": 0,
                }
            )
        rate_limited = receive_until(
            client_socket,
            lambda event: event.get("type") == "command_error"
            and event.get("code") == "RATE_LIMITED",
        )
        time.sleep(1.05)
        client_socket.send_fragmented_json({"type": "list_servers"})
        receive_until(client_socket, lambda event: event.get("type") == "server_list")
        return {
            "fragmented_command_latency_ms": fragmented_ms,
            "invalid_command_code": invalid["code"],
            "oversized_command_code": oversized_code,
            "rate_limit_code": rate_limited["code"],
            "connection_recovered_after_rate_limit": True,
        }
    finally:
        client_socket.close()


def permission_race_probe(plan: dict) -> dict:
    item = web_inventory[int(plan["session_index"])]
    cookie = item["cookie"]
    client_socket = open_websocket(int(plan["session_index"]), item)
    probe_stop = threading.Event()
    statuses: list[int] = []

    def probe_reads() -> None:
        path = (
            f"/api/channels/{quote(plan['channel'], safe='')}/messages?"
            + urlencode({"server_id": plan["server_id"], "limit": 1})
        )
        while not probe_stop.is_set():
            status, _, _ = authenticated_json_request(cookie, path)
            statuses.append(status)

    probes = [threading.Thread(target=probe_reads, daemon=True) for _ in range(2)]
    for thread in probes:
        thread.start()
    request_id = f"permission-race-{uuid.uuid4()}"
    try:
        client_socket.send_json(
            {
                "type": "lifecycle_command",
                "request_id": request_id,
                "command": {
                    "type": "leave_server",
                    "server_id": plan["server_id"],
                },
            }
        )
        outcome = receive_until(
            client_socket,
            lambda event: event.get("request_id") == request_id,
        )
        if outcome.get("type") != "lifecycle_command_succeeded":
            raise RuntimeError(f"permission-race mutation failed: {outcome}")
    finally:
        probe_stop.set()
        for thread in probes:
            thread.join(timeout=10)
        client_socket.close()
    if not statuses or any(status >= 500 for status in statuses):
        raise RuntimeError(
            f"permission-race probes did not remain bounded: statuses={statuses}"
        )
    final_path = (
        f"/api/channels/{quote(plan['channel'], safe='')}/messages?"
        + urlencode({"server_id": plan["server_id"], "limit": 1})
    )
    final_status, _, _ = authenticated_json_request(cookie, final_path)
    if final_status not in set(plan.get("denied_statuses", [403, 404])):
        raise RuntimeError(f"revoked member retained history access: status={final_status}")
    return {
        "concurrent_probe_count": len(statuses),
        "concurrent_statuses": sorted(set(statuses)),
        "post_revocation_status": final_status,
    }


def slow_upload(
    upload_index: int,
    item: dict,
    file_bytes: int,
    admitted_barrier: threading.Barrier,
    upload_release: threading.Event,
) -> dict:
    parsed = urlparse(origin)
    boundary = f"concord-qualification-{uuid.uuid4().hex}"
    preamble = (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="file"; filename="qualification-{upload_index}.bin"\r\n'
        "Content-Type: application/octet-stream\r\n\r\n"
    ).encode()
    suffix = f"\r\n--{boundary}--\r\n".encode()
    path = "/api/uploads?" + urlencode(
        {"purpose": "message", "conversation_id": item["subscriptions"][0]}
    )
    connection_class = (
        http.client.HTTPSConnection if parsed.scheme == "https" else http.client.HTTPConnection
    )
    connection_arguments = {"timeout": 300}
    if parsed.scheme == "https":
        connection_arguments["context"] = http_ssl_context or ssl.create_default_context()
    connection = connection_class(parsed.hostname, parsed.port, **connection_arguments)
    started_at = time.monotonic()
    try:
        connection.putrequest("POST", path)
        connection.putheader("Host", parsed.netloc)
        connection.putheader("Cookie", "concord_session=" + item["cookie"])
        connection.putheader("Content-Type", f"multipart/form-data; boundary={boundary}")
        connection.putheader("Content-Length", str(len(preamble) + file_bytes + len(suffix)))
        connection.endheaders()
        connection.send(preamble)
        admitted_barrier.wait(timeout=20)
        if not upload_release.wait(20):
            raise RuntimeError("maximum-size upload release deadline exceeded")
        chunk_size = 64 * 1024
        seed = hashlib.sha256(f"qualification-upload-{upload_index}".encode()).digest()
        chunk = (seed * (chunk_size // len(seed) + 1))[:chunk_size]
        remaining = file_bytes
        chunk_delay = float(os.environ.get("CONCORD_QUAL_UPLOAD_CHUNK_DELAY", "0.001"))
        while remaining:
            part = chunk[: min(remaining, len(chunk))]
            connection.send(part)
            remaining -= len(part)
            if remaining:
                time.sleep(chunk_delay)
        connection.send(suffix)
        response = connection.getresponse()
        payload = response.read()
        elapsed = time.monotonic() - started_at
        if response.status != 201:
            raise RuntimeError(
                f"maximum-size upload {upload_index} failed: status={response.status} body={payload[:200]!r}"
            )
        result = json.loads(payload)
        if int(result.get("file_size", -1)) != file_bytes:
            raise RuntimeError("maximum-size upload response reported the wrong file size")
        return {
            "index": upload_index,
            "attachment_id": result["id"],
            "file_bytes": file_bytes,
            "duration_seconds": elapsed,
            "started_monotonic": started_at,
            "finished_monotonic": time.monotonic(),
        }
    finally:
        connection.close()


def media_stress_probe() -> dict:
    file_bytes = int(os.environ["CONCORD_QUAL_MAX_UPLOAD_BYTES"])
    if file_bytes <= 0:
        raise RuntimeError("configured maximum upload bytes must be positive")
    admitted_barrier = threading.Barrier(5)
    upload_release = threading.Event()
    results: list[dict] = []
    failures: list[str] = []

    def worker(index: int) -> None:
        try:
            results.append(
                slow_upload(
                    index,
                    web_inventory[index % len(web_inventory)],
                    file_bytes,
                    admitted_barrier,
                    upload_release,
                )
            )
        except Exception as exc:
            failures.append(f"upload {index}: {exc}")

    item = web_inventory[0]
    client_socket = open_websocket(0, item)
    cursor, operation_generation = synchronize_websocket(
        client_socket, 0, session_count, item["subscriptions"], None
    )
    join_web_channels(client_socket, item)
    workers = [threading.Thread(target=worker, args=(index,)) for index in range(4)]
    request_id = f"media-stress-{uuid.uuid4()}"
    chat_ack_ms = 0.0
    try:
        for thread in workers:
            thread.start()
        admitted_barrier.wait(timeout=20)
        staging_deadline = time.monotonic() + (10 if mode == "full" else 0.8)
        while True:
            upload_telemetry = fetch_server_telemetry()
            if upload_telemetry.get("uploads") == 4:
                break
            if time.monotonic() >= staging_deadline:
                raise RuntimeError("four uploads were not concurrently staged")
            time.sleep(0.02)
        chat_started = time.monotonic()
        client_socket.send_json(
            {
                "type": "send_message",
                "operation_generation": operation_generation,
                "request_id": request_id,
                "client_message_id": request_id,
                "conversation_id": item["subscriptions"][0],
                "server_id": item["server_id"],
                "channel": "#" + item["channels"][0].rsplit("/", 1)[-1].lstrip("#"),
                "content": f"{marker_prefix}media-stress",
                "content_format": "plain",
                "reply_to": None,
                "attachment_ids": None,
                "mentions": [],
                "nonce": request_id,
            }
        )
        acknowledgement = receive_until(
            client_socket,
            lambda event: event.get("type") in ("message_ack", "command_error")
            and event.get("request_id") == request_id,
        )
        chat_ack_ms = (time.monotonic() - chat_started) * 1000
        if acknowledgement.get("type") != "message_ack":
            raise RuntimeError(
                f"chat send during four admitted uploads was rejected: {acknowledgement}"
            )
        if acknowledgement.get("client_message_id") != request_id:
            raise RuntimeError("media-stress acknowledgement lost client identity")
        collect_sample("stress")
    finally:
        upload_release.set()
        client_socket.close()
    for thread in workers:
        thread.join(timeout=330)
    if any(thread.is_alive() for thread in workers):
        raise RuntimeError("maximum-size upload worker exceeded the total request deadline")
    if failures:
        raise RuntimeError("; ".join(failures))
    if len(results) != 4:
        raise RuntimeError("four concurrent uploads did not all complete")
    latest_start = max(result["started_monotonic"] for result in results)
    earliest_finish = min(result["finished_monotonic"] for result in results)
    if latest_start >= earliest_finish:
        raise RuntimeError("upload workload did not actually overlap all four requests")
    for result in results:
        status, _, _ = authenticated_json_request(
            web_inventory[result["index"] % len(web_inventory)]["cookie"],
            f"/api/uploads/{quote(result['attachment_id'], safe='')}",
            method="DELETE",
        )
        if status not in (200, 204):
            raise RuntimeError(f"uploaded fixture cleanup failed with status {status}")
    return {
        "concurrent_uploads": 4,
        "file_bytes_each": file_bytes,
        "all_four_overlapped": True,
        "chat_acknowledged_while_four_admitted": True,
        "chat_ack_latency_ms": chat_ack_ms,
        "retained_cursor_present": bool(cursor),
        "maximum_duration_seconds": max(result["duration_seconds"] for result in results),
        "attachments_cleaned_up": 4,
    }


def restart_deduplication_probe(item: dict) -> dict:
    client_socket = open_websocket(0, item)
    cursor, operation_generation = synchronize_websocket(
        client_socket, 0, session_count, item["subscriptions"], None
    )
    join_web_channels(client_socket, item)
    request_id = f"restart-{uuid.uuid4()}"
    marker = "restartprobe" + uuid.uuid4().hex
    command = {
        "type": "send_message",
        "operation_generation": operation_generation,
        "request_id": request_id,
        "client_message_id": request_id,
        "conversation_id": item["subscriptions"][0],
        "server_id": item["server_id"],
        "channel": "#" + item["channels"][0].rsplit("/", 1)[-1].lstrip("#"),
        "content": marker,
        "content_format": "plain",
        "reply_to": None,
        "attachment_ids": None,
        "mentions": [],
        "nonce": request_id,
    }
    client_socket.send_json(command)
    restart_started = time.monotonic()
    restart = control_action("restart", timeout_seconds=30)
    restart_seconds = time.monotonic() - restart_started
    client_socket.close()
    if restart.get("ready") is not True:
        raise RuntimeError(f"target did not return ready after restart: {restart}")

    client_socket = open_websocket(0, item)
    try:
        cursor, resumed_generation = synchronize_websocket(
            client_socket, 0, session_count, item["subscriptions"], cursor
        )
        if resumed_generation != operation_generation:
            raise RuntimeError("ordinary restart unexpectedly changed operation generation")
        join_web_channels(client_socket, item)
        client_socket.send_json(command)
        acknowledgement = receive_until(
            client_socket,
            lambda event: event.get("request_id") == request_id
            and event.get("type") in ("message_ack", "command_error"),
        )
        if acknowledgement.get("type") != "message_ack":
            raise RuntimeError(f"post-restart exact retry was rejected: {acknowledgement}")
        if acknowledgement.get("client_message_id") != request_id:
            raise RuntimeError("post-restart acknowledgement lost client identity")
        search_path = "/api/search?" + urlencode(
            {
                "server_id": item["server_id"],
                "q": marker,
                "channel": "#" + item["channels"][0].rsplit("/", 1)[-1].lstrip("#"),
                "limit": 10,
            }
        )
        status, results, _ = authenticated_json_request(item["cookie"], search_path)
        if status != 200 or not isinstance(results, dict):
            raise RuntimeError("post-restart duplicate query failed")
        matching = [
            result
            for result in results.get("results", [])
            if result.get("content") == marker
        ]
        if results.get("total_count") != 1 or len(matching) != 1:
            raise RuntimeError(
                f"uncertain restart retry did not persist exactly once: {results}"
            )
        if matching[0].get("id") != str(acknowledgement.get("id")):
            raise RuntimeError("post-restart query and durable receipt identify different messages")
        return {
            "restart": True,
            "ready_after_restart": True,
            "restart_seconds": restart_seconds,
            "retry_replayed": bool(acknowledgement.get("replayed")),
            "logical_message_count": 1,
            "no_duplicate_accepted_messages": True,
            "retained_cursor_resumed": bool(cursor),
        }
    finally:
        client_socket.close()


receiver_sockets: list[socket.socket | None] = [None] * session_count


def client(index: int) -> None:
    try:
        sock = connect(index)
        sock.settimeout(1)
        send_line(sock, f"PASS {tokens[index]}")
        send_line(sock, f"NICK qual{index}")
        send_line(sock, f"USER qual{index} 0 * :qualification")
        buffer = b""
        registration_ok = False
        deadline = time.monotonic() + 20
        while not registration_ok and time.monotonic() < deadline:
            buffer += sock.recv(65536)
            lines = buffer.split(b"\n")
            buffer = lines.pop()
            registration_ok = any(b" 001 " in line for line in lines)
        if not registration_ok:
            raise RuntimeError("registration numeric 001 not received")
        for assigned_channel in receiver_channels[index]:
            send_line(sock, f"JOIN {assigned_channel}")
        receiver_sockets[index] = sock
        registered[index].set()
        while not stop.is_set():
            try:
                chunk = sock.recv(65536)
                if not chunk:
                    break
                buffer += chunk
            except TimeoutError:
                continue
            lines = buffer.split(b"\n")
            buffer = lines.pop()
            now = time.monotonic()
            for raw in lines:
                text = raw.decode(errors="replace")
                if "PING :" in text:
                    send_line(sock, "PONG :" + text.split("PING :", 1)[1].strip())
                position = text.find(marker_prefix)
                if position >= 0:
                    suffix = text[position + len(marker_prefix):].strip().split()[0]
                    if suffix.isdigit():
                        sequence = int(suffix)
                        with lock:
                            receipts.setdefault(sequence, set()).add(index)
                            first_receipt.setdefault(sequence, now)
                            if sequence_senders.get(sequence) == index:
                                sender_receipts.setdefault(sequence, set()).add(index)
                                sender_first_receipt.setdefault(sequence, now)
        sock.close()
    except Exception as exc:
        with lock:
            errors.append(f"client {index}: {exc}")


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

web_registered = [threading.Event() for _ in range(web_session_count)]
web_cursors: dict[int, str] = {}


def web_client(web_index: int, item: dict) -> None:
    global_index = session_count + web_index
    client_socket = None
    cursor = None
    operation_generation = ""
    checkpoint_interval = 5.0 if mode == "full" else 0.5
    last_checkpoint = time.monotonic()
    reconnected = False
    stalled = False
    try:
        client_socket = open_websocket(web_index, item)
        cursor, operation_generation = synchronize_websocket(
            client_socket,
            web_index,
            global_index,
            item["subscriptions"],
            None,
        )
        web_cursors[web_index] = cursor
        join_web_channels(client_socket, item)
        web_registered[web_index].set()
        while not stop.is_set():
            if web_index < reconnect_count and reconnect_requested.is_set() and not reconnected:
                client_socket.close()
                client_socket = None
                reconnect_disconnected[web_index].set()
                if not reconnect_release.wait(30):
                    raise RuntimeError("reconnect release deadline exceeded")
                client_socket = open_websocket(web_index, item)
                cursor, operation_generation = synchronize_websocket(
                    client_socket,
                    web_index,
                    global_index,
                    item["subscriptions"],
                    cursor,
                )
                web_cursors[web_index] = cursor
                join_web_channels(client_socket, item)
                reconnect_finished_at[web_index] = time.monotonic()
                reconnect_completed[web_index].set()
                reconnected = True
                last_checkpoint = time.monotonic()
                continue
            if web_index < slow_client_count and stall_requested.is_set() and not stalled:
                slow_stalled[web_index].set()
                if not stall_release.wait(60 if mode == "full" else 15):
                    raise RuntimeError("slow-client release deadline exceeded")
                try:
                    cursor, operation_generation = synchronize_websocket(
                        client_socket,
                        web_index,
                        global_index,
                        item["subscriptions"],
                        cursor,
                    )
                except (EOFError, OSError, TimeoutError):
                    client_socket.close()
                    client_socket = open_websocket(web_index, item)
                    cursor, operation_generation = synchronize_websocket(
                        client_socket,
                        web_index,
                        global_index,
                        item["subscriptions"],
                        cursor,
                    )
                    join_web_channels(client_socket, item)
                web_cursors[web_index] = cursor
                slow_finished_at[web_index] = time.monotonic()
                slow_completed[web_index].set()
                stalled = True
                last_checkpoint = time.monotonic()
                continue
            try:
                event = client_socket.receive_json()
            except TimeoutError:
                event = None
            if event is not None:
                record_web_event(web_index, global_index, event)
            if time.monotonic() - last_checkpoint >= checkpoint_interval:
                cursor, operation_generation = synchronize_websocket(
                    client_socket,
                    web_index,
                    global_index,
                    item["subscriptions"],
                    cursor,
                )
                web_cursors[web_index] = cursor
                last_checkpoint = time.monotonic()
    except Exception as exc:
        if not stop.is_set():
            with lock:
                errors.append(f"web client {web_index}: {exc}")
    finally:
        if client_socket is not None:
            client_socket.close()


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
    if query_iterations < query_worker_count:
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

with lock:
    observed = len(receipts)
    latencies = sorted((first_receipt[key] - started[key]) * 1000 for key in first_receipt if key in started)
    minimum_fanout = min((len(value) for value in receipts.values()), default=0)
    client_errors = list(errors)
if observed != message_count:
    raise SystemExit(f"message correctness failure: sent={message_count} uniquely_observed={observed}")
if len(sender_receipts) != message_count:
    raise SystemExit(
        f"acceptance acknowledgement failure: sent={message_count} sender_echoes={len(sender_receipts)}"
    )
if client_errors:
    raise SystemExit("client errors occurred during qualification: " + "; ".join(client_errors))
fanout_failures = []
for sequence, recipients in receipts.items():
    destination = sequence_channels[sequence]
    expected = expected_recipients[destination]
    if recipients != expected:
        fanout_failures.append(
            {
                "sequence": sequence,
                "channel": destination,
                "missing": sorted(expected - recipients),
                "unexpected": sorted(recipients - expected),
            }
        )
if fanout_failures:
    raise SystemExit(f"fanout correctness failure: {fanout_failures[:10]}")
required_fanout = min(expected_fanout.values())

def percentile(values: list[float], fraction: float) -> float:
    return values[min(len(values) - 1, int((len(values) - 1) * fraction))]

ack_latencies = sorted(
    (sender_first_receipt[key] - started[key]) * 1000
    for key in sender_first_receipt
    if key in started
)
if len(ack_latencies) != message_count or not query_latencies_ms:
    raise SystemExit("latency evidence is incomplete")
expected_deliveries = sum(
    len(expected_recipients[sequence_channels[sequence]])
    for sequence in range(message_count)
)
observed_deliveries = sum(len(value) for value in receipts.values())
raw_web_replay_overlap = sum(
    max(0, count - 1) for count in web_raw_receipts.values()
)
generator_sha256 = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
server_metadata = {}
if os.environ.get("CONCORD_QUAL_SERVER_METADATA"):
    server_metadata = json.load(
        open(os.environ["CONCORD_QUAL_SERVER_METADATA"], encoding="utf-8")
    )
if mode == "smoke":
    memory_bytes = 0
    try:
        for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("MemTotal:"):
                memory_bytes = int(line.split()[1]) * 1024
                break
    except OSError:
        memory_bytes = 0
    host_evidence = {
        "server_hostname": socket.gethostname(),
        "generator_hostname": socket.gethostname(),
        "server_platform": platform.platform(),
        "generator_platform": platform.platform(),
        "cpu_count": os.cpu_count() or 0,
        "memory_bytes": memory_bytes,
        "filesystem": "temporary-local-fixture",
        "storage": "temporary-local-fixture",
        "separate_load_generator": False,
    }
    input_evidence = {
        "server_sha256": os.environ["CONCORD_QUAL_SERVER_SHA256"],
        "generator_sha256": generator_sha256,
        "seed_sha256": os.environ["CONCORD_QUAL_SEED_SHA256"],
        "dataset_sha256": os.environ["CONCORD_QUAL_DATASET_SHA256"],
        "config_sha256": os.environ["CONCORD_QUAL_CONFIG_SHA256"],
        "query_mix_sha256": os.environ["CONCORD_QUAL_QUERY_MIX_SHA256"],
        "source_revision": os.environ["CONCORD_QUAL_SOURCE_REVISION"],
        "toolchain": os.environ["CONCORD_QUAL_SERVER_TOOLCHAIN"],
        "release_flags": "debug browser-fixtures bounded-smoke",
    }
    seeded_messages = int(os.environ["CONCORD_QUAL_SEEDED_MESSAGES"])
else:
    host_evidence = {
        "server_hostname": server_metadata["hostname"],
        "generator_hostname": socket.gethostname(),
        "server_platform": server_metadata["kernel"],
        "generator_platform": platform.platform(),
        "cpu_count": int(server_metadata["cpu_count"]),
        "memory_bytes": int(server_metadata["memory_bytes"]),
        "filesystem": server_metadata["filesystem"],
        "storage": server_metadata["storage"],
        "separate_load_generator": server_metadata["hostname"]
        not in {socket.gethostname(), socket.getfqdn()},
    }
    input_evidence = {
        "server_sha256": server_metadata["server_sha256"],
        "generator_sha256": generator_sha256,
        "seed_sha256": server_metadata["seed_sha256"],
        "dataset_sha256": server_metadata["dataset_sha256"],
        "config_sha256": server_metadata["config_sha256"],
        "query_mix_sha256": server_metadata["query_mix_sha256"],
        "source_revision": server_metadata["source_revision"],
        "toolchain": server_metadata["rustc"],
        "release_flags": server_metadata["release_flags"],
    }
    seeded_messages = int(server_metadata["seeded_messages"])

summary = {
    "classification": "bounded-local-smoke" if mode == "smoke" else "full-external-host-candidate",
    "started_unix_seconds": start_wall,
    "python": platform.python_version(),
    "host": host_evidence,
    "inputs": input_evidence,
    "workload": {
        "sessions": session_count + web_session_count,
        "web_sessions": web_session_count,
        "irc_sessions": session_count,
        "senders": sender_count,
        "servers": len(channels),
        "seeded_messages": seeded_messages,
        "messages_sent": message_count,
        "accepted_message_rate_per_second": message_count / steady_duration_seconds,
        "mean_fanout": expected_deliveries / message_count,
        "duration_seconds": steady_duration_seconds,
        "database_profile": server_metadata.get("database_profile", "FULL/WAL"),
    },
    "latency": {
        "commit_ack_p95_ms": percentile(ack_latencies, 0.95),
        "commit_ack_p99_ms": percentile(ack_latencies, 0.99),
        "recipient_p95_ms": percentile(latencies, 0.95),
        "recipient_p99_ms": percentile(latencies, 0.99),
    },
    "exact_fanout": {
        "passed": True,
        "sent": message_count,
        "acked": len(sender_receipts),
        "verified": observed,
        "expected_deliveries": expected_deliveries,
        "observed_deliveries": observed_deliveries,
        "duplicates": 0,
        "missing": 0,
        "minimum_fanout": minimum_fanout,
        "required_minimum_fanout": required_fanout,
        "raw_replay_overlap_deduplicated": raw_web_replay_overlap,
    },
    "history_search": {
        "passed": True,
        "history_requests": query_iterations,
        "search_requests": query_iterations,
        "history_p95_ms": percentile(sorted(history_latencies_ms), 0.95),
        "search_p95_ms": percentile(sorted(search_latencies_ms), 0.95),
        "authorization_failures": 0,
        "stable_pagination_checks": pagination_checks,
    },
    "reconnect": {
        "passed": True,
        "declared_target": reconnect_count,
        "recovered": sum(event.is_set() for event in reconnect_completed),
        "convergence_seconds": reconnect_convergence_seconds,
        "duplicates": 0,
        "gaps": 0,
    },
    "slow_abusive": {
        "passed": True,
        "slow_clients": slow_client_count,
        "closed_or_resynced": sum(event.is_set() for event in slow_completed),
        "healthy_p95_ms": percentile(latencies, 0.95),
        "healthy_p99_ms": percentile(latencies, 0.99),
        "permanently_blocked_workers": 0,
        "abusive": abusive_result,
        "permission_race": permission_race_result,
    },
    "media_provider": {
        "passed": True,
        "uploads_started": 4,
        "uploads_completed": media_result["concurrent_uploads"],
        "concurrent_uploads": media_result["concurrent_uploads"],
        "file_bytes_each": media_result["file_bytes_each"],
        "provider_failure": True,
        "provider_failures_terminal": provider_result["status"].get("delivery_state")
        == "failed",
        "core_chat_acks_during_failure": int(provider_result["chat_acknowledged"]),
        "upload": media_result,
        "provider": provider_result,
    },
    "restart_restore": {
        "passed": False,
        "restart": restart_result["restart"],
        "restore": False,
        "restart_recovered": restart_result["ready_after_restart"],
        "restore_recovered": False,
        "duplicate_publications": None,
        "accepted_messages_verified": False,
        "restart_result": restart_result,
    },
    "no_duplicate_accepted_messages": restart_result[
        "no_duplicate_accepted_messages"
    ],
    "resource": {"scope": "bounded" if mode == "smoke" else "full"},
    "marker_prefix": marker_prefix,
    "client_errors": client_errors,
    "full_acceptance_claimed": False,
    "acceptance_status": "bounded-local-smoke"
    if mode == "smoke"
    else "awaiting-restore-and-evidence-analysis",
    "unverified_acceptance_areas": [
        "dedicated 4-vCPU 8-GiB separate-host one-hour scale"
    ]
    if mode == "smoke"
    else ["fresh-instance restore and final evidence analysis"],
}
if mode == "full":
    if duration < 3600 or session_count + web_session_count < 1000 or message_count < 72000 or len(channels) != 50:
        raise SystemExit("full qualification scale or duration was reduced")
    if statistics.fmean(expected_fanout.values()) < 100:
        raise SystemExit("full qualification mean fanout target was reduced")
    if percentile(ack_latencies, 0.95) >= 250 or percentile(ack_latencies, 0.99) >= 1000:
        raise SystemExit("full qualification latency target failed")
    if reconnect_count != 200 or reconnect_convergence_seconds > 30:
        raise SystemExit("full qualification reconnect target failed")
    if slow_client_count < 100:
        raise SystemExit("full qualification slow-client target was reduced")
    if percentile(sorted(history_latencies_ms), 0.95) >= 500 or percentile(sorted(search_latencies_ms), 0.95) >= 500:
        raise SystemExit("full qualification history/search latency target failed")
(evidence / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(summary, sort_keys=True))
