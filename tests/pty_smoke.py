import fcntl
import os
import pty
import select
import struct
import subprocess
import termios
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def verify(arguments, exit_key, marker):
    master, slave = pty.openpty()
    process = None
    transcript = bytearray()
    before = termios.tcgetattr(slave)

    def drain(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.05)
            if ready:
                transcript.extend(os.read(master, 65536))

    try:
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
        process = subprocess.Popen(
            [str(ROOT / "target/debug/reswho"), "--no-containers", *arguments],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env={**os.environ, "TERM": "xterm-256color"},
            start_new_session=True,
        )
        deadline = time.monotonic() + 10
        while marker not in transcript and time.monotonic() < deadline:
            drain(0.1)
            assert process.poll() is None, transcript.decode(errors="replace")
        assert marker in transcript, "TUI did not render"
        if marker == b"Loading resources...":
            assert b"Resource" in transcript and b"Memory" in transcript, "loading must keep the table layout"
            previous = len(transcript)
            drain(0.8)
            animation = transcript[previous:]
            assert any(glyph in animation for glyph in [b"/", b"-", b"\\"]), "spinner did not advance"
            assert b"sample" not in transcript, "long sample should still be pending"
        if marker == b"sample":
            for key in [b"j", b"l", b"c", b"m", b"u", b"n", b"/reswho", b"\r", b"\x1b", b"a", b"]", b"[", b"?", b"?"]:
                os.write(master, key)
                drain(0.12)
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
            drain(0.2)
        start = time.monotonic()
        os.write(master, exit_key)
        while process.poll() is None and time.monotonic() - start < 3:
            drain(0.1)
        assert process.poll() == 0, transcript.decode(errors="replace")
        drain(0.1)
        assert termios.tcgetattr(slave) == before, "terminal mode not restored"
        assert b"\x1b[?1049h" in transcript and b"\x1b[?1049l" in transcript
        assert b"panicked" not in transcript
        print(f"PASS {arguments or ['auto TTY']} exit={exit_key!r} restored")
    finally:
        if process is not None:
            if process.poll() is None:
                process.kill()
            process.wait()
        os.close(master)
        os.close(slave)


verify([], b"q", b"sample")
verify(["--tui", "--sample-ms", "5000"], b"\x03", b"Loading resources...")
verify(["--tui", "--from", "pid:4294967295"], b"q", b"Unable to load resources")
