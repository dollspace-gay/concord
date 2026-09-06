#!/usr/bin/env python3
"""Serve secret-free, fail-closed telemetry for load/recovery qualification."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hmac
from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
from typing import Any


MAX_TOKEN_BYTES = 4096
MAX_PID_BYTES = 64
MAX_STATUS_BYTES = 1024 * 1024
MAX_PROC_NET_BYTES = 16 * 1024 * 1024


class TelemetryUnavailable(RuntimeError):
    """A required local measurement cannot be read safely."""


@dataclass(frozen=True)
class Configuration:
    server_pid_file: Path
    database: Path
    web_port: int
    irc_port: int
    token_file: Path


def bounded_read(path: Path, limit: int) -> bytes:
    try:
        with path.open("rb") as handle:
            value = handle.read(limit + 1)
    except OSError as error:
        raise TelemetryUnavailable from error
    if len(value) > limit:
        raise TelemetryUnavailable
    return value


def read_token(path: Path) -> str:
    try:
        token = bounded_read(path, MAX_TOKEN_BYTES).decode("utf-8").rstrip("\r\n")
    except UnicodeDecodeError as error:
        raise TelemetryUnavailable from error
    if not token or any(character in token for character in "\r\n\0"):
        raise TelemetryUnavailable
    return token


def read_pid(path: Path) -> int:
    try:
        text = bounded_read(path, MAX_PID_BYTES).decode("ascii").strip()
    except UnicodeDecodeError as error:
        raise TelemetryUnavailable from error
    if not text.isascii() or not text.isdigit():
        raise TelemetryUnavailable
    pid = int(text)
    if pid <= 0:
        raise TelemetryUnavailable
    return pid


def read_rss_bytes(pid: int) -> int:
    try:
        status = bounded_read(Path(f"/proc/{pid}/status"), MAX_STATUS_BYTES).decode(
            "ascii", errors="strict"
        )
    except UnicodeDecodeError as error:
        raise TelemetryUnavailable from error
    for line in status.splitlines():
        if not line.startswith("VmRSS:"):
            continue
        fields = line.split()
        if len(fields) != 3 or fields[2] != "kB" or not fields[1].isdigit():
            raise TelemetryUnavailable
        rss_bytes = int(fields[1]) * 1024
        if rss_bytes <= 0:
            raise TelemetryUnavailable
        return rss_bytes
    raise TelemetryUnavailable


def count_established_connections(ports: frozenset[int]) -> int:
    count = 0
    for path in (Path("/proc/net/tcp"), Path("/proc/net/tcp6")):
        try:
            rows = bounded_read(path, MAX_PROC_NET_BYTES).decode("ascii").splitlines()
        except UnicodeDecodeError as error:
            raise TelemetryUnavailable from error
        if not rows:
            raise TelemetryUnavailable
        for row in rows[1:]:
            fields = row.split()
            if len(fields) < 4:
                raise TelemetryUnavailable
            try:
                local_port = int(fields[1].rsplit(":", 1)[1], 16)
            except (IndexError, ValueError) as error:
                raise TelemetryUnavailable from error
            if fields[3] == "01" and local_port in ports:
                count += 1
    return count


def count_database_state(database: Path) -> tuple[int, int]:
    try:
        uri = database.resolve(strict=True).as_uri() + "?mode=ro"
        with sqlite3.connect(uri, uri=True, timeout=1.0) as connection:
            connection.execute("PRAGMA query_only=ON")
            jobs = connection.execute(
                "SELECT COUNT(*) FROM external_jobs WHERE state IN ('pending','leased')"
            ).fetchone()
            uploads = connection.execute(
                "SELECT COUNT(*) FROM attachments WHERE media_state='staging'"
            ).fetchone()
    except (OSError, sqlite3.Error) as error:
        raise TelemetryUnavailable from error
    if jobs is None or uploads is None:
        raise TelemetryUnavailable
    return int(jobs[0]), int(uploads[0])


def collect(configuration: Configuration) -> dict[str, int]:
    pid = read_pid(configuration.server_pid_file)
    rss_bytes = read_rss_bytes(pid)
    connections = count_established_connections(
        frozenset((configuration.web_port, configuration.irc_port))
    )
    jobs, uploads = count_database_state(configuration.database)
    return {
        "rss_bytes": rss_bytes,
        "connections": connections,
        "jobs": jobs,
        "uploads": uploads,
    }


class TelemetryServer(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 32

    def __init__(self, address: tuple[str, int], configuration: Configuration):
        self.configuration = configuration
        super().__init__(address, TelemetryHandler)

    def get_request(self) -> tuple[Any, Any]:
        request, client_address = super().get_request()
        request.settimeout(2.0)
        return request, client_address

    def handle_error(self, request: Any, client_address: Any) -> None:
        del request, client_address


class TelemetryHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server: TelemetryServer

    def log_message(self, format: str, *args: Any) -> None:
        del format, args

    def respond(self, status: int, value: dict[str, Any]) -> None:
        body = (json.dumps(value, separators=(",", ":"), sort_keys=True) + "\n").encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.send_header("cache-control", "no-store")
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(body)
        self.close_connection = True

    def do_GET(self) -> None:
        if self.path != "/":
            self.respond(404, {"error": "not_found"})
            return
        try:
            expected = read_token(self.server.configuration.token_file)
        except TelemetryUnavailable:
            self.respond(503, {"error": "unavailable"})
            return
        supplied = self.headers.get("authorization", "")
        if not hmac.compare_digest(supplied, f"Bearer {expected}"):
            self.respond(401, {"error": "unauthorized"})
            return
        try:
            telemetry = collect(self.server.configuration)
        except Exception:
            self.respond(503, {"error": "unavailable"})
            return
        self.respond(200, telemetry)


def request(address: tuple[str, int], token: str) -> tuple[int, dict[str, Any]]:
    connection = HTTPConnection(address[0], address[1], timeout=2)
    try:
        connection.request("GET", "/", headers={"Authorization": f"Bearer {token}"})
        response = connection.getresponse()
        body = response.read()
        return response.status, json.loads(body)
    finally:
        connection.close()


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="concord-telemetry-") as temporary:
        root = Path(temporary)
        token_file = root / "token"
        pid_file = root / "pid"
        database = root / "concord.db"
        token_file.write_text("qualification-secret\n", encoding="utf-8")
        pid_file.write_text(f"{__import__('os').getpid()}\n", encoding="ascii")
        with sqlite3.connect(database) as connection:
            connection.execute("CREATE TABLE external_jobs(state TEXT NOT NULL)")
            connection.executemany(
                "INSERT INTO external_jobs(state) VALUES(?)",
                [("pending",), ("leased",), ("completed",)],
            )
            connection.execute("CREATE TABLE attachments(media_state TEXT NOT NULL)")
            connection.executemany(
                "INSERT INTO attachments(media_state) VALUES(?)",
                [("staging",), ("ready",)],
            )
        configuration = Configuration(
            server_pid_file=pid_file,
            database=database,
            web_port=65_534,
            irc_port=65_533,
            token_file=token_file,
        )
        server = TelemetryServer(("127.0.0.1", 0), configuration)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        address = server.server_address
        try:
            status, body = request(address, "wrong-secret")
            assert status == 401 and body == {"error": "unauthorized"}

            status, body = request(address, "qualification-secret")
            assert status == 200
            assert set(body) == {"rss_bytes", "connections", "jobs", "uploads"}
            assert all(isinstance(value, int) and value >= 0 for value in body.values())
            assert body["rss_bytes"] > 0
            assert body["jobs"] == 2
            assert body["uploads"] == 1

            pid_file.write_text("not-a-pid\n", encoding="ascii")
            status, body = request(address, "qualification-secret")
            assert status == 503 and body == {"error": "unavailable"}
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
    print("PASS serve-load-recovery-telemetry self-test")
    return 0


def parse_listen(value: str) -> tuple[str, int]:
    host, separator, raw_port = value.rpartition(":")
    if not separator or not host or not raw_port.isdigit():
        raise argparse.ArgumentTypeError("listen address must be HOST:PORT")
    if host.startswith("[") and host.endswith("]"):
        host = host[1:-1]
    port = int(raw_port)
    if not 0 <= port <= 65_535:
        raise argparse.ArgumentTypeError("listen port is out of range")
    return host, port


def port(value: str) -> int:
    if not value.isdigit() or not 1 <= int(value) <= 65_535:
        raise argparse.ArgumentTypeError("port must be between 1 and 65535")
    return int(value)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--listen", type=parse_listen)
    parser.add_argument("--server-pid-file", type=Path)
    parser.add_argument("--database", type=Path)
    parser.add_argument("--web-port", type=port)
    parser.add_argument("--irc-port", type=port)
    parser.add_argument("--token-file", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    required = (
        args.listen,
        args.server_pid_file,
        args.database,
        args.web_port,
        args.irc_port,
        args.token_file,
    )
    if args.self_test:
        if any(value is not None for value in required):
            parser.error("--self-test cannot be combined with server arguments")
    elif any(value is None for value in required):
        parser.error("all server arguments are required unless --self-test is used")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.self_test:
        return run_self_test()
    configuration = Configuration(
        server_pid_file=args.server_pid_file,
        database=args.database,
        web_port=args.web_port,
        irc_port=args.irc_port,
        token_file=args.token_file,
    )
    try:
        with TelemetryServer(args.listen, configuration) as server:
            server.serve_forever(poll_interval=0.2)
    except (OSError, KeyboardInterrupt):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
