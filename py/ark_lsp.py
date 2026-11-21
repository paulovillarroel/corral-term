"""Bridge between the editor and Ark's language server.

Fresh talks LSP over stdin/stdout. Ark, the R kernel behind the console,
starts its language server when a client opens the `positron.lsp` comm, then
listens on a local TCP port. This asks the running kernel for that server and
copies bytes both ways, so the editor gets completion from the live session
(the columns of your data frames, for example) without a second R.

Usage: python ark_lsp.py <kernel connection file>   (needs jupyter_client;
`corral _lsp r` runs it with the python of the jupyter-console uv tool)
"""

from __future__ import annotations

import socket
import sys
import threading
import time
import uuid

from jupyter_client import BlockingKernelClient


def start_server(connection_file: str, timeout: float = 30) -> tuple[BlockingKernelClient, str, int]:
    kc = BlockingKernelClient()
    kc.load_connection_file(connection_file)
    kc.start_channels()
    comm_id = uuid.uuid4().hex
    kc.shell_channel.send(kc.session.msg("comm_open", {
        "comm_id": comm_id, "target_name": "positron.lsp", "data": {"ip_address": "127.0.0.1"}}))
    end = time.time() + timeout
    while time.time() < end:
        try:
            msg = kc.get_iopub_msg(timeout=1)
        except Exception:
            continue
        content = msg.get("content", {})
        if msg.get("msg_type") == "comm_msg" and content.get("comm_id") == comm_id:
            port = content.get("data", {}).get("content", {}).get("port")
            if port:
                return kc, comm_id, int(port)
    raise SystemExit("Ark no inició su servidor de lenguaje")


def main() -> None:
    kc, comm_id, port = start_server(sys.argv[1])
    sock = socket.create_connection(("127.0.0.1", port))

    def editor_to_ark() -> None:
        while True:
            data = sys.stdin.buffer.read1(65536)
            if not data:
                break
            sock.sendall(data)
        sock.shutdown(socket.SHUT_WR)

    threading.Thread(target=editor_to_ark, daemon=True).start()
    try:
        while True:
            data = sock.recv(65536)
            if not data:
                break
            sys.stdout.buffer.write(data)
            sys.stdout.buffer.flush()
    finally:
        try:
            kc.shell_channel.send(kc.session.msg("comm_close", {"comm_id": comm_id, "data": {}}))
            kc.stop_channels()
        except Exception:
            pass


if __name__ == "__main__":
    main()
