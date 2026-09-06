"""Websocket for load/recovery qualification."""
from __future__ import annotations
import json
import os
import base64
import hashlib
import re
import socket
import ssl
import struct
import time
import uuid
from urllib.parse import quote, urlencode, urlparse
from .state import http_ssl_context, lock, marker_prefix, origin, receipts, session_count, web_logical_receipts, web_raw_receipts
from .irc_transport import source_address

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
