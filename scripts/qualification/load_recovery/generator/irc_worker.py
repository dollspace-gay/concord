"""Irc worker for load/recovery qualification."""
from __future__ import annotations
import time
from .state import errors, first_receipt, lock, marker_prefix, receipts, receiver_channels, receiver_sockets, registered, sender_first_receipt, sender_receipts, sequence_senders, stop, tokens
from .irc_transport import connect, send_line

def client(index: int) -> None:
    try:
        sock = connect(index)
        sock.settimeout(1)
        send_line(sock, f"PASS {tokens[index]}")
        send_line(sock, f"NICK qual{index}")
        send_line(sock, f"USER qual{index} 0 * :qualification")
        buffer = b""
        registration_ok = False
        deadline = time.monotonic() + 20
        while not registration_ok and time.monotonic() < deadline:
            buffer += sock.recv(65536)
            lines = buffer.split(b"\n")
            buffer = lines.pop()
            registration_ok = any(b" 001 " in line for line in lines)
        if not registration_ok:
            raise RuntimeError("registration numeric 001 not received")
        for assigned_channel in receiver_channels[index]:
            send_line(sock, f"JOIN {assigned_channel}")
        receiver_sockets[index] = sock
        registered[index].set()
        while not stop.is_set():
            try:
                chunk = sock.recv(65536)
                if not chunk:
                    break
                buffer += chunk
            except TimeoutError:
                continue
            lines = buffer.split(b"\n")
            buffer = lines.pop()
            now = time.monotonic()
            for raw in lines:
                text = raw.decode(errors="replace")
                if "PING :" in text:
                    send_line(sock, "PONG :" + text.split("PING :", 1)[1].strip())
                position = text.find(marker_prefix)
                if position >= 0:
                    suffix = text[position + len(marker_prefix):].strip().split()[0]
                    if suffix.isdigit():
                        sequence = int(suffix)
                        with lock:
                            receipts.setdefault(sequence, set()).add(index)
                            first_receipt.setdefault(sequence, now)
                            if sequence_senders.get(sequence) == index:
                                sender_receipts.setdefault(sequence, set()).add(index)
                                sender_first_receipt.setdefault(sequence, now)
        sock.close()
    except Exception as exc:
        with lock:
            errors.append(f"client {index}: {exc}")
