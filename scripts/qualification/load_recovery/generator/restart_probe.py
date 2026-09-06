"""Restart probe for load/recovery qualification."""
from __future__ import annotations
import time
import uuid
from urllib.parse import quote, urlencode, urlparse
from .state import session_count
from .websocket import join_web_channels, open_websocket, receive_until, synchronize_websocket
from .http import authenticated_json_request, control_action

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
