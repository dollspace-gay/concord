"""Permission probe for load/recovery qualification."""
from __future__ import annotations
import threading
import uuid
from urllib.parse import quote, urlencode, urlparse
from .state import web_inventory
from .websocket import open_websocket, receive_until
from .http import authenticated_json_request

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
