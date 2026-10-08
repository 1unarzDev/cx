#!/usr/bin/env python3
"""Bounded physical wheel probe: report mouse events, never key text or screen content."""
import json
import os
import re
import select
import signal
import sys
import termios
import time
import tty


def main():
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise SystemExit("Run directly in the terminal where CX skips rows.")
    print("Move the pointer here. Scroll down exactly 10 ticks, pausing between ticks,")
    print("then up exactly 10 ticks. Press Ctrl+C to finish (60-second limit).")
    print("Only mouse reports and arrow counts are collected; no typed text is saved.")
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    def interrupted(_signal, _frame):
        raise SystemExit("Wheel probe interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    started = time.monotonic()
    pending = b""
    wheel = []
    arrows = 0
    pattern = re.compile(rb"\x1b\[<(\d+);(\d+);(\d+)([Mm])")
    try:
        tty.setraw(fd)
        # Match CX's alternate-screen and crossterm mouse modes.
        os.write(sys.stdout.fileno(), b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h")
        os.write(sys.stdout.fileno(), b"Scroll 10 ticks down, then 10 up. Ctrl+C finishes.\r\n")
        while time.monotonic() - started < 60 and len(wheel) < 256:
            if not select.select([fd], [], [], 0.1)[0]:
                continue
            incoming = os.read(fd, 4096)
            if not incoming or b"\x03" in incoming:
                break
            pending += incoming
            while pending:
                match = pattern.match(pending)
                if match:
                    button = int(match[1])
                    if button & 64:
                        wheel.append({"elapsed_ms": round((time.monotonic() - started) * 1000, 2),
                                      "button": button, "ending": match[4].decode()})
                    pending = pending[match.end():]
                elif pending.startswith((b"\x1b[A", b"\x1b[B")):
                    arrows += 1
                    pending = pending[3:]
                elif pending in (b"\x1b", b"\x1b["):
                    break
                elif pending.startswith(b"\x1b[<") and len(pending) < 64 and not re.search(rb"[Mm]", pending[3:]):
                    break
                else:
                    pending = pending[1:]
    finally:
        try:
            os.write(sys.stdout.fileno(), b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l")
        finally:
            termios.tcsetattr(fd, termios.TCSANOW, saved)
    print(json.dumps({"wheel_reports": wheel, "arrow_reports": arrows}, indent=2))


if __name__ == "__main__":
    main()
