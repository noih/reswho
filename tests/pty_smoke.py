import fcntl
import os
import pty
import json
import re
import tempfile
import select
import signal
import struct
import sys
import subprocess
import termios
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
config_owner = tempfile.TemporaryDirectory(prefix="reswho-pty-")
config_root = Path(config_owner.name)
print(f"OWNED PTY config root {config_root}", flush=True)


def full_frame(slave, process, transcript, drain):
    height, width, _, _ = struct.unpack("HHHH", fcntl.ioctl(slave, termios.TIOCGWINSZ, b"\0" * 8))
    start = len(transcript)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width ^ 1, 0, 0))
    os.kill(process.pid, signal.SIGWINCH)
    drain(0.2)
    return re.sub(rb"\x1b\[[0-9;?]*[A-Za-z]", b" ", transcript[start:])


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
            env={**os.environ, "TERM": "xterm-256color", "XDG_CONFIG_HOME": str(config_root)},
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
            os.write(master, b"p\x1b[C")
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
            os.write(master, b"r")
            deadline = time.monotonic() + 2
            while True:
                manual_frame = full_frame(slave, process, transcript, drain)
                if b"refreshing" in manual_frame:
                    break
                assert time.monotonic() < deadline, f"manual refresh frame: {manual_frame!r}"
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
                assert "Resource ↑".encode() in transcript, "initial sort option was not applied"
            for key in [b"j", b"l", b"c", b"m", b"u", b"n", b"/reswho", b"\r", b"\x1b", b"a", b"]", b"[", b"?", b"?"]:
                os.write(master, key)
                drain(0.12)
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
            drain(0.2)
        if details:
            os.write(master, b"\x1b")
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



def verify_priority(config):
    config = config.resolve()
    master, slave = pty.openpty()
    process = None
    original = config.read_bytes()
    transcript = bytearray()
    def drain(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.05)
            if ready:
                transcript.extend(os.read(master, 65536))
    def send(keys, seconds=0.25):
        os.write(master, keys)
        drain(seconds)
    try:
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 110, 0, 0))
        process = subprocess.Popen([str(ROOT / "target/debug/reswho"), "--tui", "--no-containers", "--config", str(config)], stdin=slave, stdout=slave, stderr=slave, env={**os.environ, "TERM":"xterm-256color"}, start_new_session=True)
        deadline = time.monotonic() + 10
        while b"sample" not in transcript and time.monotonic() < deadline:
            drain(0.1)
        assert b"sample" in transcript
        send(b"pe")
        frame = full_frame(slave, process, transcript, drain)
        assert b"Type" not in frame and b"Version" not in frame and b"Author" not in frame, frame
        assert b"app_bundle" in frame and b"Fixed" in frame and b"Required" not in frame, frame
        send(b"jjj ")
        send(b"\x1b")
        assert config.read_bytes() == original, "inventory cancel wrote config"
        send(b"e\x1b[Hjjj ")
        send(b"\x13")
        assert json.loads(config.read_text())["disabled_builtins"] == ["project"]
        frame = full_frame(slave, process, transcript, drain)
        assert b"Space Enable" in frame and b"* project" not in frame and b"Sources" in frame, frame
        send(b"\x1b")
        send(b"e ")
        frame = full_frame(slave, process, transcript, drain)
        assert re.search(rb"Sources.*Ownership", frame), frame
        for label in [rb"ancestor", rb"bin", rb"Ctrl-S\s+Save"]:
            assert re.search(label, frame), f"priority inventory frame: {frame!r}"
        send(b"j")
        send(b"\x1b[1;2A")
        send(b"\x13")
        saved = json.loads(config.read_text())
        assert saved["disabled_builtins"] == [], saved
        assert saved["source_priority"][:5] == ["app_bundle", "bin", "command", "ancestor", "project"], saved
        frame = full_frame(slave, process, transcript, drain)
        assert b"Shift" in frame and b"Sources" in frame and b"* ancestor" not in frame and b"* project" not in frame, frame
        send(b"\x1b")
        send(b"e\t")
        frame = full_frame(slave, process, transcript, drain)
        assert b"Ownership" in frame, frame
        send(b"jjjj\x1b[1;2A\x13")
        saved = json.loads(config.read_text())
        assert saved["ownership_priority"] == ["extensions.vm.app", "extensions.vm.helper", "extensions.vm.system", "project", "app", "work", "bin"], saved
        frame = full_frame(slave, process, transcript, drain)
        assert b"Rank / Name" in frame and b"Ownership" in frame, frame
        send(b"\x1b")
        offline = subprocess.run([str(ROOT / "target/debug/reswho"), "--config", str(config), "settings", "--json"], check=True, capture_output=True)
        assert json.loads(offline.stdout)["source_priority"] == saved["source_priority"]
        send(b"e")
        frame = full_frame(slave, process, transcript, drain)
        assert b"Sources" in frame and b"Ownership" in frame, f"priority reopen frame: {frame!r}"
        send(b"\r")
        frame = full_frame(slave, process, transcript, drain)
        assert not re.search(rb"Source\s+details", frame), frame
        send(b"\x1b[C")
        frame = full_frame(slave, process, transcript, drain)
        assert re.search(rb"Source\s+details", frame), f"priority details frame: {frame!r}"
        assert b"ID: project" in frame and b"Capabilities: classify" in frame, frame
        assert all(field not in frame for field in (b"Author:", b"Version:", b"Provenance:", b"Origin:")), frame
        send(b"\x1b[D")
        frame = full_frame(slave, process, transcript, drain)
        assert b"Sources" in frame, f"priority details back frame: {frame!r}"
        send(b"\x1b[1;2B")
        config.write_text("malformed")
        send(b"\x13")
        frame = full_frame(slave, process, transcript, drain)
        assert b"Config changed on disk; reopen Settings or Sources before saving" in frame, frame.decode(errors="replace")
        assert config.read_text() == "malformed", "failed save replaced config"
        send(b"sqe")
        assert process.poll() is None, "priority inventory q quit"
        frame = full_frame(slave, process, transcript, drain)
        assert b"Sources" in frame, f"priority local keys frame: {frame!r}"
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 10, 36, 0, 0))
        send(b"\x1b[1;2A")
        frame = full_frame(slave, process, transcript, drain)
        assert re.search(rb"Ctrl-S\s+Save", frame), f"compact priority frame: {frame!r}"
        send(b"\x1b")
        frame = full_frame(slave, process, transcript, drain)
        assert b"Sources" not in frame, f"priority discard frame: {frame!r}"
        config.write_text(json.dumps(saved))
        send(b"q", 0.4)
        process.wait(timeout=4)
        assert process.returncode == 0
        print("PASS priority reorder/save/restart/error/36x10/navigation")
    finally:
        if process is not None:
            if process.poll() is None:
                process.kill()
            process.wait()
        os.close(master)
        os.close(slave)
        config.write_bytes(original)


def verify_schedule(mode):
    master, slave = pty.openpty()
    process = None
    fixture = None
    before = termios.tcgetattr(slave)
    transcript = bytearray()
    config = config_root / f"{mode}.json"
    config.write_text(json.dumps({"version": 1, "settings": {
        "sampling.interval_ms": 400,
        "refresh.interval_ms": 600 if mode == "auto" else 60000,
    }}))

    def drain(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.05)
            if ready:
                transcript.extend(os.read(master, 65536))
            assert process.poll() is None, transcript.decode(errors="replace")

    try:
        fixture = subprocess.Popen(["/bin/sh", "-c", "read -r owned_input"], stdin=subprocess.PIPE,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
        arguments = [str(ROOT / "target/debug/reswho"), "--no-containers", "--config", str(config),
                     "--from", f"pid:{fixture.pid}"]
        if mode == "pause":
            arguments.extend(["--set", "refresh.interval_ms=600"])
        process = subprocess.Popen(arguments, stdin=slave, stdout=slave, stderr=slave,
                                   env={**os.environ, "TERM": "xterm-256color"}, start_new_session=True)
        deadline = time.monotonic() + 10
        while b"sample" not in transcript and time.monotonic() < deadline:
            drain(0.1)
        plain = re.sub(rb"\x1b\[[0-9;?]*[A-Za-z]", b" ", transcript)
        match = re.search(rb"sample\s+(\d+)ms", plain)
        assert match and int(match[1]) >= 400, f"config sampling was not applied: {plain[-1000:]!r}"
        if mode == "pause":
            os.write(master, b"p")
            drain(0.2)
            assert b"paused" in transcript
        fixture.stdin.close()
        fixture.wait(timeout=3)
        if mode == "auto":
            deadline = time.monotonic() + 4
            while b"STALE" not in transcript and time.monotonic() < deadline:
                drain(0.1)
            assert b"STALE" in transcript, "configured automatic refresh did not run"
        else:
            drain(1.8)
            assert b"STALE" not in transcript, "refresh ignored pause or configured wait"
            os.write(master, b"r")
            deadline = time.monotonic() + 4
            while b"STALE" not in transcript and time.monotonic() < deadline:
                drain(0.1)
            assert b"STALE" in transcript, "manual refresh must bypass pause and wait"
        os.write(master, b"q")
        process.wait(timeout=3)
        assert process.returncode == 0
        assert termios.tcgetattr(slave) == before
        print(f"PASS config sampling and {mode} refresh restored")
    finally:
        if process is not None:
            if process.poll() is None:
                process.kill()
            process.wait()
        if fixture is not None:
            if fixture.poll() is None:
                fixture.kill()
            fixture.wait()
            if not fixture.stdin.closed:
                fixture.stdin.close()
        os.close(master)
        os.close(slave)
        config.unlink(missing_ok=True)

def verify_settings():
    root = config_root.resolve() / "settings-popup"
    root.mkdir(mode=0o700)
    config = root / "config.json"
    config.write_text(json.dumps({"version": 1, "settings": {"sampling.interval_ms": 1000, "refresh.interval_ms": 60000, "unknown.keep": "untouched"}}))
    master, slave = pty.openpty()
    process = None
    transcript = bytearray()
    before = termios.tcgetattr(slave)

    def drain(seconds=0.3):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.05)
            if ready:
                transcript.extend(os.read(master, 65536))

    def send(keys):
        os.write(master, keys)
        drain()
        assert process.poll() is None, transcript.decode(errors="replace")

    frame_width = 120
    frame_height = 30

    def frame_text():
        nonlocal frame_width
        frame_width = (37 if frame_width == 36 else 36) if frame_width <= 40 else (121 if frame_width == 120 else 120)
        start = len(transcript)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", frame_height, frame_width, 0, 0))
        os.kill(process.pid, signal.SIGWINCH)
        drain()
        return re.sub(rb"\x1b\[[0-9;?]*[A-Za-z]", b" ", transcript[start:])

    try:
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
        process = subprocess.Popen([str(ROOT / "target/debug/reswho"), "--tui", "--no-containers", "--config", str(config), "--set", "refresh.interval_ms=45000"], stdin=slave, stdout=slave, stderr=slave, env={**os.environ, "TERM": "xterm-256color"}, start_new_session=True)
        deadline = time.monotonic() + 10
        while b"sample" not in transcript and time.monotonic() < deadline:
            drain(0.1)
            assert process.poll() is None, transcript.decode(errors="replace")
        assert b"sample" in transcript, transcript.decode(errors="replace")
        send(b"ps")
        assert b"Settings" in transcript
        initial = config.read_bytes()
        send(b"\x1b[H\x1b[B\r")
        locked = frame_text()
        assert b"45000" in locked and b"locked" in locked and re.search(rb"CLI\s+override\s+is\s+locked", locked), locked
        send(b"\x12")
        assert re.search(rb"CLI\s+override\s+is\s+locked", frame_text())
        assert config.read_bytes() == initial
        send(b"\x1b[B\r\x15" + b"0\r")
        assert re.search(rb"Invalid\s+value", frame_text())
        send(b"\x1b[Z\t\x13")
        invalid = frame_text()
        assert re.search(rb"Invalid\s+value", invalid) and b"[0_]" in invalid, invalid
        assert b"[Cancel]" not in invalid and b"[Save]" not in invalid and b"Tab Controls" not in invalid
        assert config.read_bytes() == initial
        send(b"\x1b\x1b")
        assert config.read_bytes() == initial
        send(b"s\x1b[H\x1b[B")
        send(b"\x1b[B\r\x15" + b"0\r")
        send(b"\x1b")
        browse = frame_text()
        assert b"45000" in browse and b"1000" in browse and b"Settings" in browse, browse
        assert config.read_bytes() == initial
        send(b"\x1b")
        closed = frame_text()
        assert not re.search(rb"Ctrl-S\s+Save", closed) and re.search(rb"s\s+Settings", closed), closed
        assert config.read_bytes() == initial
        send(b"s\r\x15" + b"400\r")
        assert config.read_bytes() == initial
        send(b"\x13")
        saved = json.loads(config.read_text())
        assert saved["settings"]["sampling.interval_ms"] == 400, saved
        assert saved["settings"]["refresh.interval_ms"] == 60000, saved
        send(b"s\x1b[H\x1b[B")
        retained = frame_text()
        assert b"45000" in retained and b"locked" in retained, retained
        send(b"\x1b[B\x1b")
        assert saved["settings"]["unknown.keep"] == "untouched"
        report = subprocess.run([str(ROOT / "target/debug/reswho"), "settings", "--config", str(config), "--json"], check=True, capture_output=True, timeout=5)
        assert json.loads(report.stdout)["settings"]["sampling.interval_ms"]["value"] == 400
        send(b"r")
        drain(1)
        plain = frame_text()
        sample = re.search(rb"sample\s+(\d+)ms", plain)
        assert sample and 400 <= int(sample[1]) < 1000, plain[-2000:]
        assert b"paused" in transcript
        mtime = config.stat().st_mtime_ns
        send(b"s\x13")
        assert config.stat().st_mtime_ns == mtime
        send(b"s\x12\x13")
        assert "sampling.interval_ms" not in json.loads(config.read_text())["settings"]
        send(b"s\r\x15" + b"500\r")
        other = json.loads(config.read_text())
        other["settings"]["sampling.interval_ms"] = 800
        config.write_text(json.dumps(other))
        send(b"\x13")
        assert re.search(rb"changed\s+on\s+disk", frame_text())
        assert json.loads(config.read_text())["settings"]["sampling.interval_ms"] == 800
        send(b"\x1b")
        send(b"s\r\x15" + b"600\r")
        config.write_text("malformed")
        send(b"\x13")
        assert re.search(rb"changed\s+on\s+disk", frame_text())
        assert config.read_text() == "malformed"
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 10, 36, 0, 0))
        os.kill(process.pid, signal.SIGWINCH)
        drain()
        frame_width = 36
        frame_height = 10
        os.kill(process.pid, signal.SIGWINCH)
        drain()
        compact_start = len(transcript)
        send(b"\x1b")
        send(b"s")
        for width in [37, 36]:
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 10, width, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            drain()
        compact = re.sub(rb"\x1b\[[0-9;?]*[A-Za-z]", b" ", transcript[compact_start:])
        for label in [rb"Ctrl-S\s+Save", rb"Ctrl-R\s+Reset", rb"Esc\s+discard", rb"Time\s+between"]:
            assert re.search(label, compact), compact[-3000:]
        send(b"\x1b[6~")
        assert b"Key:" not in frame_text(), "list PageDown must not open details"
        send(b"\x1b[C")
        pages = bytearray()
        views = []
        for _ in range(8):
            views.append(frame_text())
            pages.extend(views[-1])
            send(b"\x1b[6~")
        assert views[0] != views[1], "details PageDown must move the visible content"
        for label in [b"Key:", b"Default:", b"Constraints:", b"max=5000", b"Value:", b"Back", b"Scroll", b"private backup"]:
            assert label in pages, pages
        send(b"\x1b")
        assert re.search(rb"Time\s+between", frame_text())
        assert b"Ctrl-S" in frame_text(), "first Esc must return to Settings list"
        send(b"\x1b[C")
        send(b"\x1b[D")
        assert re.search(rb"Time\s+between", frame_text())
        send(b"\x13")
        assert json.loads(config.read_text())["version"] == 1
        backups = list(config.parent.glob(".config.json.recovery-*.bak"))
        assert len(backups) == 1 and backups[0].read_bytes() == b"malformed"
        assert backups[0].stat().st_mode & 0o777 == 0o600
        send(b"e")
        assert b"Sources" in frame_text()
        send(b"sqe")
        inventory = frame_text()
        assert b"Sources" in inventory and b"Settings" not in inventory, inventory
        send(b"\r")
        assert not re.search(rb"Source\s+details", frame_text())
        send(b"d")
        assert re.search(rb"Source\s+details", frame_text())
        send(b"sqe")
        assert re.search(rb"Source\s+details", frame_text())
        send(b"\x1b")
        assert b"Sources" in frame_text()
        send(b"\x1b")
        assert b"Sources" not in frame_text()
        os.write(master, b"q")
        process.wait(timeout=5)
        assert process.returncode == 0
        assert termios.tcgetattr(slave) == before
        print("PASS settings PTY CLI-lock/save-preserves-override/draft/invalid/cancel/field-Esc/popup-Esc/save/reload/nochange/reset/conflict/failure/resize/pause/extensions-isolation/back")
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        os.close(master)
        os.close(slave)


def verify_help():
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

    def frame_text():
        assert process.poll() is None
        return re.sub(rb"\s+", b" ", full_frame(slave, process, transcript, drain)).decode(errors="replace")

    def page(name, content):
        frame = frame_text()
        assert re.search(r"\[\s*" + name + r"\s*\]", frame), frame
        assert content in frame, frame

    def main_frame():
        frame = frame_text()
        assert re.search(r"→\s+Open", frame) and re.search(r"q\s+Quit", frame), frame
        assert not re.search(r"\[\s*(Controls|Panels|Metrics)\s*\]", frame), frame

    try:
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        process = subprocess.Popen(
            [str(ROOT / "target/debug/reswho"), "--tui", "--no-containers", "--set", "refresh.interval_ms=60000"],
            stdin=slave, stdout=slave, stderr=slave,
            env={**os.environ, "TERM": "xterm-256color", "XDG_CONFIG_HOME": str(config_root)},
            start_new_session=True,
        )
        drain(0.4)
        main_frame()
        os.write(master, b"?")
        drain(0.15)
        page("Controls", "Navigation")
        os.write(master, b"\t")
        drain(0.15)
        page("Panels", "Settings")
        os.write(master, b"\x1b[Z")
        drain(0.15)
        page("Controls", "Navigation")
        os.write(master, b"\t\t")
        drain(0.15)
        page("Metrics", "Host CPU")
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 10, 36, 0, 0))
        os.kill(process.pid, signal.SIGWINCH)
        drain(0.2)
        os.write(master, b"\x1b[F")
        drain(0.15)
        page("Metrics", "one query at a time")
        os.write(master, b"\x1b")
        drain(0.2)
        main_frame()
        os.write(master, b"?")
        drain(0.15)
        page("Controls", "Navigation")
        os.write(master, b"?")
        drain(0.15)
        main_frame()
        os.write(master, b"q")
        process.wait(timeout=5)
        assert process.returncode == 0
        assert termios.tcgetattr(slave) == before
        print("PASS Help PTY Tab/Shift-Tab/36x10/End/Esc/?/q/termios (full redraw content smoke)")
    finally:
        try:
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        finally:
            os.close(master)
            os.close(slave)


if "--help-only" in sys.argv:
    try:
        verify_help()
    finally:
        config_owner.cleanup()
        assert not config_root.exists()
        print("PASS Help PTY child reaped, handles closed and temp root removed")
    sys.exit(0)


if "--settings-only" in sys.argv:
    try:
        verify_settings()
    finally:
        config_owner.cleanup()
        assert not config_root.exists()
        print("PASS settings PTY handles closed and temp root removed")
    sys.exit(0)


if "--schedule-only" in sys.argv:
    try:
        for mode in ["auto", "wait", "pause"]:
            verify_schedule(mode)
    finally:
        config_owner.cleanup()
        assert not config_root.exists()
        print("PASS scheduling PTYs/children closed and config temp removed")
    sys.exit(0)

try:
    if not any(flag in sys.argv for flag in ["--extensions-only", "--priority-only"]):
        verify(["--sort", "name"], b"q", b"sample")
        verify(["--tui", "--set", "sampling.interval_ms=5000"], b"\x03", b"Loading resources...")
        verify(["--tui", "--from", "pid:4294967295"], b"q", b"Unable to load resources")

        fixture = subprocess.Popen(
            ["/bin/sh", "-c", "read -r owned_fixture_input", "fixture", "--token", "fixture-private-value"],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        try:
            verify(["--tui", "--from", f"pid:{fixture.pid}", "--set", "sampling.interval_ms=1000"], b"q", b"sample", details=True)
        finally:
            fixture.terminate()
            fixture.wait()
            fixture.stdin.close()
    owned_root = config_root / "bundled"
    owned_root.mkdir()
    config = owned_root / "config.json"
    config.write_text(json.dumps({"version":1,"settings":{"sampling.interval_ms":400}}))
    verify_priority(config)
    if not any(flag in sys.argv for flag in ["--extensions-only", "--priority-only"]):
        verify_settings()
        for mode in ["auto", "wait", "pause"]:
            verify_schedule(mode)
finally:
    config_owner.cleanup()
    assert not config_root.exists(), "owned PTY config temp was not removed"
    print("PASS owned PTYs/children closed and config temp removed")
