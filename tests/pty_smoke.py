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


def verify(arguments, exit_key, marker, details=False):
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
        while (marker not in transcript or b"\x1b[?2026l" not in transcript) and time.monotonic() < deadline:
            drain(0.1)
            assert process.poll() is None, transcript.decode(errors="replace")
        assert marker in transcript, "TUI did not render"
        sync_start = transcript.index(b"\x1b[?2026h")
        alternate = transcript.index(b"\x1b[?1049h")
        sync_end = transcript.index(b"\x1b[?2026l")
        loading = transcript.index(b"Loading resources...")
        assert sync_start < alternate < loading < sync_end, "initial frame must include screen entry and loading in one synchronized update"
        first_frame = transcript[alternate:sync_end]
        for label in [b"Host", b"CPU", b"Memory", b"Path"]:
            assert label in first_frame, f"initial synchronized frame missing {label!r}"
        if marker == b"Loading resources...":
            assert b"Resource" in transcript and b"Memory" in transcript, "loading must keep the table layout"
            previous = len(transcript)
            drain(0.8)
            animation = transcript[previous:]
            assert any(glyph in animation for glyph in [b"/", b"-", b"\\"]), "spinner did not advance"
            assert b"sample" not in transcript, "long sample should still be pending"
        if marker == b"sample" and details:
            os.write(master, b"pd")
            opened = len(transcript)
            deadline = time.monotonic() + 5
            while b"[filtered]" not in transcript[opened:] and time.monotonic() < deadline:
                drain(0.1)
                assert process.poll() is None
            rendered = transcript[opened:]
            assert b"Process details" in rendered, "details did not open"
            assert b"Loading command" in rendered, "details loading was not shown"
            assert b"[filtered]" in rendered and b"--token" in rendered, "filtered command missing"
            assert b"fixture-private-value" not in transcript, "command exposed sensitive value"
            manual = len(transcript)
            os.write(master, b"r")
            deadline = time.monotonic() + 2
            while b"efreshing" not in transcript[manual:] and time.monotonic() < deadline:
                drain(0.05)
            assert b"efreshing" in transcript[manual:], "manual refresh feedback missing"
            drain(1.2)
            for key in [b"j", b"\x1b[6~", b"\x1b[5~", b"k"]:
                os.write(master, key)
                drain(0.1)
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 14, 80, 0, 0))
            drain(0.2)
            os.write(master, b"\x1b")
            drain(0.2)
            reopened = len(transcript)
            os.write(master, b"d")
            deadline = time.monotonic() + 2
            while b"Loading command" not in transcript[reopened:] and time.monotonic() < deadline:
                drain(0.05)
            assert b"Loading command" in transcript[reopened:], "reopening must query fresh command data"
        if marker == b"sample" and not details:
            if "--sort" in arguments:
                assert b"Sort NAME" in transcript, "initial sort option was not applied"
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
        assert transcript.count(b"\x1b[?2026h") == transcript.count(b"\x1b[?2026l"), "synchronized update left open"
        print(f"PASS {arguments or ['auto TTY']} exit={exit_key!r} restored")
    finally:
        if process is not None:
            if process.poll() is None:
                process.kill()
            process.wait()
        os.close(master)
        os.close(slave)


verify(["--sort", "name"], b"q", b"sample")
verify(["--tui", "--sample-ms", "5000"], b"\x03", b"Loading resources...")
verify(["--tui", "--from", "pid:4294967295"], b"q", b"Unable to load resources")

fixture = subprocess.Popen(
    ["/bin/sh", "-c", "read -r owned_fixture_input", "fixture", "--token", "fixture-private-value"],
    stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
)
try:
    verify(["--tui", "--from", f"pid:{fixture.pid}", "--sample-ms", "1000"], b"q", b"sample", details=True)
finally:
    fixture.terminate()
    fixture.wait()
    fixture.stdin.close()
