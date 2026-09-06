"""Web worker for load/recovery qualification."""
from __future__ import annotations
import time
from .state import errors, lock, mode, reconnect_completed, reconnect_count, reconnect_disconnected, reconnect_finished_at, reconnect_release, reconnect_requested, session_count, slow_client_count, slow_completed, slow_finished_at, slow_stalled, stall_release, stall_requested, stop, web_cursors, web_registered
from .websocket import join_web_channels, open_websocket, record_web_event, synchronize_websocket

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
