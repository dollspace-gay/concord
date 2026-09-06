"""Queries for load/recovery qualification."""
from __future__ import annotations
from urllib.parse import quote, urlencode, urlparse
from .state import history_latencies_ms, lock, query_failures, query_latencies_ms, query_plan, query_stop, search_latencies_ms, web_inventory
from . import state
from .http import authenticated_json_request

def query_worker(worker_index: int) -> None:

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
                state.query_iterations += 1
                state.pagination_checks += int(bool(continuation))
            query_stop.wait(float(plan.get("interval_seconds", 2.0)))
    except Exception as exc:
        with lock:
            query_failures.append(f"query worker {worker_index}: {exc}")
        query_stop.set()
