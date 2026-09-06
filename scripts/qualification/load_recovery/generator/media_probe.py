"""Media probe for load/recovery qualification."""
from __future__ import annotations
import json
import os
import hashlib
import http.client
import ssl
import threading
import time
import uuid
from urllib.parse import quote, urlencode, urlparse
from .state import http_ssl_context, marker_prefix, mode, origin, session_count, web_inventory
from .websocket import join_web_channels, open_websocket, receive_until, synchronize_websocket
from .http import authenticated_json_request
from .telemetry import collect_sample, fetch_server_telemetry

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
