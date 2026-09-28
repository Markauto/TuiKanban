#!/usr/bin/env python3
"""Exercise the actual terminal event loop on Unix; no third-party modules needed."""
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/kanban").resolve())


def session(board, quit_key, exercise, startup=False):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 110, 0, 0))
    before = termios.tcgetattr(slave)

    def attach():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    process = subprocess.Popen(
        ([binary] if startup else [binary, "--file", str(board)]), stdin=slave, stdout=slave, stderr=slave,
        preexec_fn=attach, env={**os.environ, "TERM": "xterm-256color"},
    )
    output = bytearray()

    def drain(seconds=0.15):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([master], [], [], max(0, deadline - time.monotonic()))[0]:
                try:
                    chunk = os.read(master, 65536)
                    if not chunk:
                        break
                    output.extend(chunk)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break

    def send(keys):
        os.write(master, keys)
        drain()

    def wait_for(predicate):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            drain(0.05)
            if predicate():
                return
            assert process.poll() is None, output.decode(errors="replace")
        raise AssertionError("TUI did not reach expected state: " + output.decode(errors="replace"))

    try:
        wait_for(lambda: b"KANBAN" in output)
        if startup:
            wait_for(lambda: b"Recent" in output)
        if exercise:
            send(b"b")
            wait_for(lambda: b"Recent" in output)
            if os.environ.get("KANBAN_CAPTURE"):
                Path(os.environ["KANBAN_CAPTURE"]).write_bytes(output)
            send(b"n")
            send(b"Switched board\t")
            # Replace the suggested path using Home and Delete.
            send(b"\x1b[H" + b"\x1b[3~" * 300)
            other = board.parent / "other.json"
            send(str(other).encode() + b"\r")
            wait_for(lambda: other.exists())
            send(b"nOther card\x13")
            wait_for(lambda: len(json.loads(other.read_text())["cards"]) == 1)
            assert not json.loads(board.read_text())["cards"]
            send(b"bo" + str(board).encode() + b"\r")
            send(b"n")
            send("Terminal λ".encode())
            send(b"\tDescription from terminal\tcli,tui\t2026-12-01\t\x1b[C\t\x1b[C")
            send(b"\x13")  # Ctrl+S
            wait_for(lambda: len(json.loads(board.read_text())["cards"]) == 1)
            card = json.loads(board.read_text())["cards"][0]
            assert card["title"] == "Terminal λ", card
            assert card["column"] == "In Progress", card
            assert card["priority"] == "high", card
            assert card["description"] == "Description from terminal", card
            assert card["tags"] == ["cli", "tui"], card
            assert card["due"] == "2026-12-01", card
            send(b"H")
            wait_for(lambda: json.loads(board.read_text())["cards"][0]["column"] == "Todo")
            send(b"e\x1b[HUpdated ")  # editor Home, insert at beginning
            send(b"\x13")
            wait_for(lambda: json.loads(board.read_text())["cards"][0]["title"] == "Updated Terminal λ")
            send(b"a")
            wait_for(lambda: json.loads(board.read_text())["cards"][0]["archived"])
            send(b"va")
            wait_for(lambda: not json.loads(board.read_text())["cards"][0]["archived"])
            send(b"NBlocked\r")
            wait_for(lambda: "Blocked" in json.loads(board.read_text())["columns"])
            send(b"E\x1b[F!\r")
            wait_for(lambda: "Blocked!" in json.loads(board.read_text())["columns"])
            send(b"[s")
            wait_for(lambda: json.loads(board.read_text()).get("stacked_columns") == ["Blocked!"])
            send(b"v?")
            send(b"\x1b")
            # Resize exercises the compact editor and minimum-size rendering.
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 12, 35, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            send(b"e")
            send(b"\x1b")
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 5, 15, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            drain()
        send(quit_key)
        assert process.wait(timeout=5) == 0, output.decode(errors="replace")
        drain()
        assert termios.tcgetattr(slave) == before, "Terminal settings were not restored"
        assert b"\x1b[?1049h" in output and b"\x1b[?1049l" in output, "Alternate screen was not restored"
        assert b"panicked" not in output
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)


with tempfile.TemporaryDirectory(prefix="kanban-pty-") as directory:
    os.environ["XDG_DATA_HOME"] = str(Path(directory) / "data")
    os.environ.pop("KANBAN_FILE", None)
    board = Path(directory) / "board.json"
    subprocess.run([binary, "--file", str(board), "init", "Terminal test"], check=True, capture_output=True)
    session(board, b"q", True)
    session(board, b"\x03", False)
    session(board, b"q", False, startup=True)
print("PTY smoke passed: board create/open/switch, startup picker, cards, resize, q/Ctrl+C, terminal restoration")
