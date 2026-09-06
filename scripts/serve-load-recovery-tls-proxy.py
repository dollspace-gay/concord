#!/usr/bin/env python3
"""Small TLS terminator for an isolated Concord qualification target.

Each accepted front-end connection uses a distinct loopback source address for
the backend connection. Concord therefore sees independent transport peers and
still exercises its normal per-address admission policy when the generator
opens 200 long-lived WebSockets through this target-local proxy.
"""

from __future__ import annotations

import argparse
import asyncio
import contextlib
import itertools
import ssl


def split_address(value: str) -> tuple[str, int]:
    host, separator, port = value.rpartition(":")
    if not separator or not host or not port.isdigit():
        raise argparse.ArgumentTypeError("address must be HOST:PORT")
    return host, int(port)


async def copy_stream(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    try:
        while data := await reader.read(64 * 1024):
            writer.write(data)
            await writer.drain()
    except (ConnectionError, asyncio.CancelledError):
        return
    finally:
        with contextlib.suppress(ConnectionError):
            writer.write_eof()


async def serve(arguments: argparse.Namespace) -> None:
    listen_host, listen_port = arguments.listen
    upstream_host, upstream_port = arguments.upstream
    sequence = itertools.count()

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(arguments.certificate, arguments.private_key)

    async def handle(
        front_reader: asyncio.StreamReader, front_writer: asyncio.StreamWriter
    ) -> None:
        index = next(sequence)
        third = 64 + (index // (250 * 250)) % 63
        fourth_group = (index // 250) % 250
        final = index % 250 + 1
        source = f"127.{third}.{fourth_group}.{final}"
        try:
            back_reader, back_writer = await asyncio.wait_for(
                asyncio.open_connection(
                    upstream_host,
                    upstream_port,
                    local_addr=(source, 0),
                ),
                timeout=5,
            )
        except (OSError, TimeoutError):
            front_writer.close()
            await front_writer.wait_closed()
            return
        front_to_back = asyncio.create_task(copy_stream(front_reader, back_writer))
        back_to_front = asyncio.create_task(copy_stream(back_reader, front_writer))
        await asyncio.wait(
            (front_to_back, back_to_front), return_when=asyncio.FIRST_COMPLETED
        )
        for task in (front_to_back, back_to_front):
            task.cancel()
        await asyncio.gather(front_to_back, back_to_front, return_exceptions=True)
        back_writer.close()
        front_writer.close()
        await asyncio.gather(
            back_writer.wait_closed(), front_writer.wait_closed(), return_exceptions=True
        )

    server = await asyncio.start_server(
        handle,
        listen_host,
        listen_port,
        ssl=context,
        backlog=1024,
        limit=128 * 1024,
    )
    async with server:
        await server.serve_forever()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", required=True, type=split_address)
    parser.add_argument("--upstream", required=True, type=split_address)
    parser.add_argument("--certificate", required=True)
    parser.add_argument("--private-key", required=True)
    arguments = parser.parse_args()
    asyncio.run(serve(arguments))


if __name__ == "__main__":
    main()
