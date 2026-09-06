"""Irc transport for load/recovery qualification."""
from __future__ import annotations
import socket
import ssl
from .state import host, port, source_ips, tls_ca_file, tls_server_name

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
