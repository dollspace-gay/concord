"""Provider probe for load/recovery qualification."""
from __future__ import annotations
import time
import uuid
from .state import marker_prefix, mode, session_count
from .websocket import join_web_channels, open_websocket, receive_until, synchronize_websocket
from .http import control_action

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
