import concurrent.futures
import datetime
import http.client
import json
import math
import os
import re
import signal
import socket
import subprocess
import time


MAX_BYTES = 8 * 1024 * 1024
MAX_CONTAINERS = 8
WORKERS = 4
DEADLINE_SECONDS = 60


def remaining(deadline, maximum):
    seconds = min(maximum, deadline - time.monotonic())
    if seconds <= 0:
        raise RuntimeError("measurement deadline exceeded")
    return seconds


def docker(args, deadline):
    process = subprocess.Popen(
        ["docker", *args],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
        env={**os.environ, "SSH_ASKPASS_REQUIRE": "never", "SSH_ASKPASS": "/usr/bin/false"},
    )
    try:
        stdout, stderr = process.communicate(timeout=remaining(deadline, 10))
        if process.returncode or stderr or len(stdout) > MAX_BYTES:
            raise RuntimeError("Docker CLI failed or reported diagnostics; output withheld")
        return stdout.decode("utf-8")
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        process.stdout.close()
        process.stderr.close()


def source(deadline):
    context = os.environ.get("DOCKER_CONTEXT")
    host = os.environ.get("DOCKER_HOST")
    if context or not host:
        context = context or docker(["context", "show"], deadline).strip()
        host = json.loads(docker([
            "context", "inspect", "--format", "{{json .Endpoints.docker.Host}}", "--", context
        ], deadline))
        args = [f"--context={context}"]
    else:
        args = [f"--host={host}"]
    if not host.startswith("unix://"):
        raise RuntimeError("this experiment only supports the selected Unix endpoint")
    return host[7:], args


def request(socket_path, path, deadline):
    connection = http.client.HTTPConnection("localhost")
    response = None
    try:
        transport = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        connection.sock = transport
        transport.settimeout(remaining(deadline, 5))
        transport.connect(socket_path)
        connection.request("GET", path)
        response = connection.getresponse()
        if response.status != 200:
            raise RuntimeError(f"Engine HTTP {response.status}; body withheld")
        body = bytearray()
        while True:
            transport.settimeout(remaining(deadline, 5))
            chunk = response.read1(min(65536, MAX_BYTES + 1 - len(body)))
            if not chunk:
                break
            body.extend(chunk)
            if len(body) > MAX_BYTES:
                raise RuntimeError("Engine output limit exceeded")
        return json.loads(body), time.time()
    finally:
        if response is not None:
            response.close()
        connection.close()


def read_time(value):
    value = re.sub(r"(\.\d{6})\d+", r"\1", value)
    return datetime.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()


def cpu(previous, current):
    before = previous.get("cpu_usage", {}).get("total_usage")
    after = current.get("cpu_usage", {}).get("total_usage")
    system_before = previous.get("system_cpu_usage")
    system_after = current.get("system_cpu_usage")
    cores = current.get("online_cpus") or len(current.get("cpu_usage", {}).get("percpu_usage", []))
    if None in (before, after, system_before, system_after) or not cores:
        return None
    if after < before or system_after <= system_before:
        return None
    value = (after - before) / (system_after - system_before) * cores * 100
    return value if math.isfinite(value) else None


def bounds(values):
    return [round(min(values), 3), round(max(values), 3)] if values else None


def identity(record):
    state = record.get("State", {})
    return record.get("Id"), state.get("StartedAt"), state.get("Running"), state.get("Restarting")


def main():
    deadline = time.monotonic() + DEADLINE_SECONDS
    socket_path, cli_args = source(deadline)
    version, _ = request(socket_path, "/version", deadline)
    api = version["ApiVersion"]
    if version.get("Os") != "linux" or tuple(map(int, api.split("."))) < (1, 41):
        raise RuntimeError("requires Linux Engine API 1.41 or later")
    prefix = f"/v{api}"
    print(json.dumps({"engine": {key: version.get(key) for key in ["Version", "ApiVersion", "GitCommit", "Os", "Arch"]}, "workers": WORKERS, "sample_ms": 250}), flush=True)
    modes = ["cli", "api_default", "api_pair"]
    with concurrent.futures.ThreadPoolExecutor(max_workers=WORKERS) as executor:
        for round_index in range(3):
            inventory, _ = request(socket_path, prefix + "/containers/json?all=false", deadline)
            ids = sorted(item["Id"] for item in inventory if item.get("State") == "running")[:MAX_CONTAINERS]
            if not ids or any(len(value) != 64 or any(ch not in "0123456789abcdef" for ch in value) for value in ids):
                raise RuntimeError("no eligible running containers or invalid IDs")

            def batch(suffix):
                return list(executor.map(lambda container_id: request(socket_path, prefix + f"/containers/{container_id}/{suffix}", deadline), ids))

            for mode in modes[round_index:] + modes[:round_index]:
                before = [identity(record) for record, _ in batch("json")]
                started = time.monotonic()
                values, intervals, ages = [], [], []
                memory_samples = 0
                empty_previous = None
                if mode == "cli":
                    output = docker([*cli_args, "stats", "--no-stream", "--no-trunc", "--format", '{{json .}}', "--", *ids], deadline)
                    records = [json.loads(line) for line in output.splitlines() if line.strip()]
                    if sorted(record["ID"] for record in records) != ids:
                        raise RuntimeError("CLI returned a different container set")
                    for record in records:
                        value = float(record["CPUPerc"].rstrip("%"))
                        if math.isfinite(value) and value >= 0:
                            values.append(value)
                else:
                    if mode == "api_default":
                        records = batch("stats?stream=false")
                        previous = [(record.get("precpu_stats", {}), record.get("preread")) for record, _ in records]
                    else:
                        first = batch("stats?stream=false&one-shot=true")
                        time.sleep(min(0.25, remaining(deadline, 0.25)))
                        records = batch("stats?stream=false&one-shot=true")
                        previous = [(record.get("cpu_stats", {}), record.get("read")) for record, _ in first]
                        empty_previous = sum(not record.get("precpu_stats", {}).get("system_cpu_usage") for record, _ in first)
                        if [record.get("id") for record, _ in first] != ids:
                            raise RuntimeError("first sample identities do not match")
                    if [record.get("id") for record, _ in records] != ids:
                        raise RuntimeError("sample identities do not match")
                    for (record, received), (prior_cpu, prior_read) in zip(records, previous):
                        value = cpu(prior_cpu, record.get("cpu_stats", {}))
                        elapsed = (read_time(record["read"]) - read_time(prior_read)) * 1000 if prior_read else 0
                        if value is not None and elapsed > 0:
                            values.append(value)
                            intervals.append(elapsed)
                            ages.append((received - read_time(record["read"])) * 1000)
                        memory_samples += isinstance(record.get("memory_stats", {}).get("usage"), int)
                elapsed_ms = (time.monotonic() - started) * 1000
                after = [identity(record) for record, _ in batch("json")]
                stable = before == after and all(item[1] and item[2] and not item[3] for item in after)
                complete = stable and len(values) == len(ids) and (mode == "cli" or memory_samples == len(ids))
                print(json.dumps({
                    "round": round_index + 1, "mode": mode, "elapsed_ms": round(elapsed_ms, 3),
                    "containers": len(ids), "identity_stable": stable, "complete": complete,
                    "cpu_valid": len(values), "cpu_percent_range": bounds(values),
                    "sample_interval_ms_range": bounds(intervals), "sample_age_ms_range": bounds(ages),
                    "memory_samples": memory_samples if mode != "cli" else None,
                    "first_oneshot_without_previous": empty_previous,
                }), flush=True)
                if not complete:
                    raise RuntimeError("incomplete comparison; stop after the reported result")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        details = str(error) if type(error) is RuntimeError else "measurement stopped; raw responses withheld"
        print(json.dumps({"error_type": type(error).__name__, "details": details}), flush=True)
        raise SystemExit(1)
