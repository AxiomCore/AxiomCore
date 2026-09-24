from pathlib import Path
import errno
import fcntl
import os
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unittest
from unittest.mock import patch

import ctl
import progress


class BuildProgressTests(unittest.TestCase):
    def test_elapsed_and_log_names_are_stable(self):
        self.assertEqual(progress.elapsed_label(3661.9), "01:01:01")
        with tempfile.TemporaryDirectory(prefix="axiom-progress-test-") as temporary:
            directory = Path(temporary)
            first = progress.next_log_path(directory, "ui-host-web")
            self.assertEqual(first.name, "build-ui-host-web-1.log")
            first.write_text("previous attempt")
            self.assertEqual(progress.next_log_path(directory, "ui-host-web").name,
                             "build-ui-host-web-2.log")

    def test_noninteractive_build_keeps_full_log_and_fails_closed(self):
        with tempfile.TemporaryDirectory(prefix="axiom-progress-test-") as temporary:
            directory = Path(temporary)
            output = []
            with patch.object(progress.sys.stdin, "isatty", return_value=False), \
                    patch.object(progress, "_write_terminal", side_effect=output.append):
                log = directory / "success.log"
                progress.run((sys.executable, "-c", "print('building')"), cwd=directory,
                             env={}, label="test", log_path=log)
                self.assertIn(b"building", log.read_bytes())
                self.assertIn(b"building", b"".join(output))
                failure = directory / "failure.log"
                with self.assertRaisesRegex(ctl.ReleaseError, "build exited 7"):
                    progress.run((sys.executable, "-c", "print('failed'); raise SystemExit(7)"),
                                 cwd=directory, env={}, label="test", log_path=failure)
                self.assertIn(b"failed", failure.read_bytes())
                with self.assertRaises(FileExistsError):
                    progress.run((sys.executable, "-c", "print('overwrite')"), cwd=directory,
                                 env={}, label="test", log_path=log)

    def test_pty_attach_and_detach_keeps_build_running(self):
        with tempfile.TemporaryDirectory(prefix="axiom-progress-test-") as temporary:
            directory = Path(temporary)
            log = directory / "interactive.log"
            master, slave = os.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
            program = (
                "import os,sys; from pathlib import Path; "
                f"sys.path.insert(0,{str(Path(progress.__file__).parent)!r}); "
                "import progress; "
                "progress.run((sys.executable,'-c',"
                "'import time; print(\"hello\",flush=True); time.sleep(1.5); print(\"done\",flush=True)'),"
                f"cwd=Path({str(directory)!r}),env=os.environ.copy(),label='probe',"
                f"log_path=Path({str(log)!r}))"
            )
            environment = os.environ.copy()
            environment["TERM"] = "xterm-256color"
            process = subprocess.Popen([sys.executable, "-B", "-c", program],
                                       stdin=slave, stdout=slave, stderr=slave,
                                       env=environment, start_new_session=True)
            os.close(slave)
            try:
                read_until(master, b"probe", 5)
                os.write(master, b"l")
                attached = read_until(master, b"Live logs:", 5)
                self.assertIn(b"Live logs:", attached)
                os.write(master, bytes([progress.DETACH_KEY]))
                detached = read_until(master, b"Returned to progress", 5)
                self.assertIn(b"Returned to progress", detached)
                read_until(master, b"Built probe", 8)
                self.assertEqual(process.wait(timeout=8), 0)
                self.assertIn(b"hello", log.read_bytes())
                self.assertIn(b"done", log.read_bytes())
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)

    def test_pty_cancel_stops_child_without_traceback(self):
        with tempfile.TemporaryDirectory(prefix="axiom-progress-test-") as temporary:
            directory = Path(temporary)
            log = directory / "cancel.log"
            master, slave = os.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
            program = (
                "import os,sys\nfrom pathlib import Path\n"
                f"sys.path.insert(0,{str(Path(progress.__file__).parent)!r})\n"
                "import ctl,progress\n"
                "try:\n"
                " progress.run((sys.executable,'-c','import time; time.sleep(30)'),"
                f"cwd=Path({str(directory)!r}),env=os.environ.copy(),label='cancel-probe',"
                f"log_path=Path({str(log)!r}))\n"
                "except ctl.ReleaseError as error:\n print(error)\n raise SystemExit(2)\n"
            )
            environment = os.environ.copy()
            environment["TERM"] = "xterm-256color"
            process = subprocess.Popen([sys.executable, "-B", "-c", program],
                                       stdin=slave, stdout=slave, stderr=slave,
                                       env=environment, start_new_session=True)
            os.close(slave)
            try:
                read_until(master, b"cancel-probe", 5)
                os.write(master, bytes([progress.CANCEL_KEY]))
                output = read_until(master, b"build interrupted", 10)
                self.assertNotIn(b"Traceback", output)
                self.assertEqual(process.wait(timeout=10), 2)
                self.assertTrue(log.is_file())
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)


def read_until(descriptor: int, marker: bytes, timeout: float) -> bytes:
    output = bytearray()
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        readable, _, _ = select.select([descriptor], [], [], 0.1)
        if readable:
            try:
                output.extend(os.read(descriptor, 65536))
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                break
            if marker in output:
                return bytes(output)
    raise AssertionError(f"terminal output did not contain {marker!r}: {bytes(output)[-1000:]!r}")


if __name__ == "__main__":
    unittest.main()
