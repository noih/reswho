import fcntl
import hashlib
import os
import pty
import json
import re
import tempfile
import select
import signal
import struct
import sys
import shutil
import subprocess
import termios
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
config_owner = tempfile.TemporaryDirectory(prefix="reswho-pty-")
config_root = Path(config_owner.name)


def verify(arguments, exit_key, marker, details=False, external=False, resource=False):
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
        if resource:
            os.write(master, b"pd")
            drain(0.4)
            assert b"Resource details" in transcript and b"Capacity" in transcript and b"bytes" in transcript, "resource typed details missing"
            assert b"Temperature" in transcript and b"celsius" in transcript, "unknown resource metric missing"
            os.write(master, b"\x1b")
            drain(0.2)
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
            if external:
                assert b"Shared" in transcript and b"Example" in transcript, "external parsed details missing"
                assert b"fixture-secret-value" not in transcript, "external details exposed sensitive field"
                counter = Path(arguments[arguments.index("--config") + 1]).parent / "calls"
                calls_before = counter.read_text()
                opened_status = len(transcript)
                os.write(master, b"\x1b")
                drain(0.2)
                os.write(master, b"e")
                drain(0.3)
                assert b"Extensions" in transcript[opened_status:] and b"Collect" in transcript[opened_status:], "extension inventory missing"
                assert counter.read_text() == calls_before, "opening status executed provider"
                os.write(master, b"sqe")
                drain(0.2)
                assert process.poll() is None, "inventory q quit"
                os.write(master, b"j\r")
                drain(0.3)
                assert b"Extension details" in transcript[opened_status:], "provider details missing"
                os.write(master, b"sqe")
                drain(0.2)
                assert process.poll() is None, "extension details q quit"
                os.write(master, b"\x1b")
                drain(0.2)
                os.write(master, b"\x1b")
                drain(0.3)
                assert counter.read_text() == calls_before, "returning from status executed provider"
                os.write(master, b"d")
                drain(1.2)
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
            arguments.extend(["--refresh-ms", "600"])
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
        process = subprocess.Popen([str(ROOT / "target/debug/reswho"), "--tui", "--no-containers", "--config", str(config)], stdin=slave, stdout=slave, stderr=slave, env={**os.environ, "TERM": "xterm-256color"}, start_new_session=True)
        deadline = time.monotonic() + 10
        while b"sample" not in transcript and time.monotonic() < deadline:
            drain(0.1)
            assert process.poll() is None, transcript.decode(errors="replace")
        assert b"sample" in transcript, transcript.decode(errors="replace")
        send(b"ps")
        assert b"Settings" in transcript
        initial = config.read_bytes()
        send(b"\x1b[B\r\x15" + b"0\r")
        assert re.search(rb"Invalid\s+value", frame_text())
        send(b"\x1b[Z\r")
        assert re.search(rb"Invalid\s+value", frame_text())
        assert config.read_bytes() == initial
        send(b"\t\t\r")
        assert config.read_bytes() == initial
        send(b"s\x1b[H\r\x15" + b"45000\r")
        send(b"\x1b[B\r\x15" + b"0\r")
        send(b"\x1b")
        browse = frame_text()
        assert b"45000" in browse and b"1000" in browse and b"Settings" in browse, browse
        assert config.read_bytes() == initial
        send(b"\x1b")
        assert b"Settings" not in frame_text()
        assert config.read_bytes() == initial
        send(b"s\r\x15" + b"400\r")
        assert config.read_bytes() == initial
        send(b"\x13")
        saved = json.loads(config.read_text())
        assert saved["settings"]["sampling.interval_ms"] == 400, saved
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
        assert b"malformed" in frame_text()
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
        pages = bytearray()
        send(b"\x1b[6~")
        for _ in range(8):
            pages.extend(frame_text())
            send(b"\x1b[6~")
        for label in [b"Key:", b"Default:", b"Constraints:", b"max=5000", b"Value:", b"Ctrl-S", b"Save", b"Ctrl-R", b"Reset", b"Esc", b"discard"]:
            assert label in pages, pages
        for _ in range(8):
            send(b"\x1b[5~")
        assert re.search(rb"Time\s+between", frame_text())
        send(b"\x1b")
        send(b"e")
        assert b"Extensions" in frame_text()
        send(b"sqe")
        inventory = frame_text()
        assert b"Extensions" in inventory and b"Settings" not in inventory, inventory
        send(b"\r")
        assert re.search(rb"Extension\s+details", frame_text())
        send(b"sqe")
        assert re.search(rb"Extension\s+details", frame_text())
        send(b"\x1b")
        assert b"Extensions" in frame_text()
        send(b"\x1b")
        assert b"Extensions" not in frame_text()
        os.write(master, b"q")
        process.wait(timeout=5)
        assert process.returncode == 0
        assert termios.tcgetattr(slave) == before
        print("PASS settings PTY draft/invalid/cancel/field-Esc/popup-Esc/save/reload/nochange/reset/conflict/failure/resize/pause/extensions-isolation/back")
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        os.close(master)
        os.close(slave)


if "--settings-only" in sys.argv:
    try:
        verify_settings()
    finally:
        config_owner.cleanup()
        assert not config_root.exists()
        print("PASS settings PTY handles closed and temp root removed")
    sys.exit(0)



try:
    if "--extensions-only" not in sys.argv:
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
    example_root = config_root / "example"
    example_root.mkdir()
    (example_root / ".reswho-example").write_text("")
    config = example_root / "config.json"
    config.write_text(json.dumps({"version":1,"settings":{"sampling.interval_ms":400}}))
    package = example_root / "extensions" / "example"
    package.mkdir(parents=True)
    payload = package / "example-native"
    shutil.copy2(ROOT / "target/debug/examples/extension_example", payload)
    counter = example_root / "calls"
    source = example_root / "entry.c"
    source.write_text('#include <stdio.h>\n#include <unistd.h>\nint main(void){FILE *f=fopen(' + json.dumps(str(counter)) + ',"a");fputs("called\\n",f);fclose(f);execl(' + json.dumps(str(payload)) + ',"example-native",(char*)0);return 127;}')
    subprocess.run(["cc", str(source), "-o", str(package / "provider")], check=True, timeout=10)
    manifest = json.loads((ROOT / "examples/extensions/example.json").read_text())
    manifest["executable"] = "provider"
    (package / "manifest.json").write_text(json.dumps(manifest))
    broken = example_root / "extensions" / "broken"
    broken.mkdir()
    (broken / "manifest.json").write_text("{bad fixture")
    subprocess.run([str(ROOT / "target/debug/reswho"), "--config", str(config), "extensions", "--json"], check=True, capture_output=True)
    assert not counter.exists(), "offline discovery executed native provider"
    external_fixture = subprocess.Popen(["/bin/sh", "-c", "read -r owned_external_input", "fixture", "--token", "fixture-private-value"], cwd=example_root, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        result = subprocess.run([str(ROOT / "target/debug/reswho"), "--config", str(config), "--no-containers", "--json", "--from", f"pid:{external_fixture.pid}"], check=True, capture_output=True, timeout=10)
        snapshot = json.loads(result.stdout)
        assert snapshot["schema"] == "reswho/11"
        assert snapshot["root"]["attribution"]["extension"]["provider"] == "example"
        assert snapshot["root"]["process"]["roles"][0]["label"] == "Workspace worker"
        assert snapshot["root"]["process"]["extension_details"][0]["fields"][0]["value"] == 400
        assert b"fixture-secret-value" not in result.stdout
        calls = [record for record in snapshot["extensions"] if record["provider"] == "example" and record["phase"] in ["classify", "details"]]
        assert len(calls) == 2 and all(record["result"] == "success" and record["duration_ms"] is not None for record in calls)
        assert len(counter.read_text().splitlines()) == 3, "JSON profiling executed extra providers"
        markdown = subprocess.run([str(ROOT / "target/debug/reswho"), "--config", str(config), "--no-containers", "--md", "--profile", "--from", f"pid:{external_fixture.pid}"], check=True, capture_output=True, timeout=10)
        assert b"Extension timings" in markdown.stdout and b"Duration ms" in markdown.stdout
        assert b"Extension broken / read" in markdown.stdout
        assert len(counter.read_text().splitlines()) == 6, "Markdown profiling executed extra providers"
        verify(["--tui", "--config", str(config), "--from", f"pid:{external_fixture.pid}", "--sample-ms", "1000"], b"q", b"sample", details=True, external=True)
        resource_ref = "group:" + hashlib.sha256(b"resource\0example\0example-volume").hexdigest()[:16]
        verify(["--tui", "--config", str(config), "--from", resource_ref, "--sample-ms", "200"], b"q", b"sample", resource=True)
    finally:
        external_fixture.terminate()
        external_fixture.wait()
        external_fixture.stdin.close()
    if "--extensions-only" not in sys.argv:
        for mode in ["auto", "wait", "pause"]:
            verify_schedule(mode)
finally:
    config_owner.cleanup()
    assert not config_root.exists(), "owned PTY config temp was not removed"
    print("PASS owned PTYs/children closed and config temp removed")
