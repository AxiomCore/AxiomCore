#!/usr/bin/env python3
"""Live release-build progress with an attachable terminal and SSD log."""

from __future__ import annotations

import errno
import fcntl
import os
from pathlib import Path
import select
import signal
import struct
import subprocess
import sys
import termios
import time
import tty

import ctl


SPINNER = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
DETACH_KEY = 29  # Ctrl-]
CANCEL_KEY = 3   # Ctrl-C while the terminal is in raw mode
TAIL_LIMIT = 131072


def next_log_path(directory: Path, component_id: str) -> Path:
    for attempt in range(1, 10000):
        path = directory / f"build-{component_id}-{attempt}.log"
        if not path.exists():
            return path
    raise ctl.ReleaseError(f"too many build attempts for {component_id}: {directory}")


def elapsed_label(seconds: float) -> str:
    elapsed = int(seconds)
    hours, rest = divmod(elapsed, 3600)
    minutes, seconds = divmod(rest, 60)
    return f"{hours:02}:{minutes:02}:{seconds:02}"


def _write_terminal(data: bytes) -> None:
    view = memoryview(data)
    while view:
        view = view[os.write(sys.stdout.fileno(), view):]


def _set_pty_size(master: int, terminal: int) -> None:
    try:
        size = os.get_terminal_size(terminal)
        fcntl.ioctl(master, termios.TIOCSWINSZ,
                    struct.pack("HHHH", size.lines, size.columns, 0, 0))
    except (OSError, AttributeError):
        pass


def _stop_child(process: subprocess.Popen) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGINT)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def _render_status(label: str, started: float, frame: int, log_path: Path) -> None:
    width = os.get_terminal_size(sys.stdout.fileno()).columns
    line = (f"{SPINNER[frame % len(SPINNER)]} {label}  "
            f"{elapsed_label(time.monotonic() - started)}  "
            "[l] live logs  [Ctrl-C] cancel")
    _write_terminal(("\r\x1b[2K" + line[:max(0, width - 1)]).encode("utf-8", "replace"))


def _show_tail(tail: bytearray, label: str, log_path: Path) -> None:
    _write_terminal(b"\r\x1b[2K\n")
    _write_terminal(f"Live logs: {label} | full log: {log_path} | Ctrl-] returns to progress\n".encode())
    if tail:
        lines = bytes(tail).splitlines(keepends=True)
        _write_terminal(b"".join(lines[-100:]))
        if not tail.endswith(b"\n"):
            _write_terminal(b"\n")


def run(command: tuple[str, ...], *, cwd: Path, env: dict[str, str],
        label: str, log_path: Path) -> None:
    """Run one build command; never overwrite a prior attempt log."""
    log_path.parent.mkdir(parents=True, exist_ok=True)
    if not (sys.stdin.isatty() and sys.stdout.isatty()
            and os.environ.get("TERM", "dumb") != "dumb"):
        # CI keeps ordinary streaming output. Interactive local builds use
        # the attachable PTY below; both modes write logs to the release SSD.
        with log_path.open("xb") as log:
            process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                       start_new_session=True)
            try:
                assert process.stdout is not None
                for chunk in iter(lambda: os.read(process.stdout.fileno(), 65536), b""):
                    log.write(chunk)
                    _write_terminal(chunk)
                code = process.wait()
            except KeyboardInterrupt as error:
                _stop_child(process)
                raise ctl.ReleaseError(f"build interrupted; log: {log_path}") from error
            finally:
                if process.stdout is not None:
                    process.stdout.close()
        if code:
            raise ctl.ReleaseError(f"build exited {code}; log: {log_path}")
        return

    terminal = sys.stdin.fileno()
    original = termios.tcgetattr(terminal)
    master, slave = os.openpty()
    _set_pty_size(master, terminal)
    process = None
    started = time.monotonic()
    tail = bytearray()
    attached = False
    cancelled = False
    frame = 0
    last_frame = 0.0
    try:
        with log_path.open("xb") as log:
            process = subprocess.Popen(command, cwd=cwd, env=env, stdin=slave,
                                       stdout=slave, stderr=slave, start_new_session=True)
            os.close(slave)
            slave = -1
            tty.setraw(terminal)
            while True:
                now = time.monotonic()
                if not attached and now - last_frame >= 0.1:
                    _render_status(label, started, frame, log_path)
                    last_frame = now
                    frame += 1
                ready, _, _ = select.select([master, terminal], [], [], 0.1)
                if master in ready:
                    try:
                        data = os.read(master, 65536)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        data = b""
                    if data:
                        log.write(data)
                        log.flush()
                        tail.extend(data)
                        if len(tail) > TAIL_LIMIT:
                            del tail[:-TAIL_LIMIT]
                        if attached:
                            _write_terminal(data)
                    elif process.poll() is not None:
                        break
                if terminal in ready:
                    keys = os.read(terminal, 1024)
                    for key in keys:
                        if key == CANCEL_KEY:
                            cancelled = True
                            _stop_child(process)
                            break
                        if attached and key == DETACH_KEY:
                            attached = False
                            _write_terminal(b"\r\nReturned to progress.\r\n")
                        elif not attached and key in (ord("l"), ord("L")):
                            attached = True
                            _show_tail(tail, label, log_path)
                        elif attached:
                            os.write(master, bytes([key]))
                if cancelled:
                    break
                if process.poll() is not None and master not in ready:
                    # Read any final PTY bytes on the next iteration.
                    readable, _, _ = select.select([master], [], [], 0)
                    if not readable:
                        break
            code = process.wait()
    except KeyboardInterrupt as error:
        cancelled = True
        if process is not None:
            _stop_child(process)
        raise ctl.ReleaseError(f"build interrupted; log: {log_path}") from error
    finally:
        if process is not None and process.poll() is None:
            _stop_child(process)
        termios.tcsetattr(terminal, termios.TCSADRAIN, original)
        if slave >= 0:
            os.close(slave)
        os.close(master)
        _write_terminal(b"\r\x1b[0m\x1b[2K\n")
    if cancelled:
        raise ctl.ReleaseError(f"build interrupted; log: {log_path}")
    if code:
        raise ctl.ReleaseError(f"build exited {code}; log: {log_path}")
    print(f"Built {label} in {elapsed_label(time.monotonic() - started)}. Log: {log_path}")


def command_runner(directory: Path, component_id: str, position: int, total: int):
    """Adapt progress.run to ctl.build_component's command-runner signature."""
    def execute(*args: str, cwd: Path, env: dict[str, str], capture: bool = False) -> bytes:
        if capture:
            raise ctl.ReleaseError("interactive build runner cannot capture output")
        run(tuple(args), cwd=cwd, env=env,
            label=f"{component_id} ({position}/{total})",
            log_path=next_log_path(directory, component_id))
        return b""
    return execute
