"""Abuse probe for load/recovery qualification."""
from __future__ import annotations
import json
import time
import uuid
from .websocket import open_websocket, receive_until

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
