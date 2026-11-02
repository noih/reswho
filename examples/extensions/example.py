#!/usr/bin/python3
import json
import sys
from pathlib import Path

request = json.load(sys.stdin)
provider = request["provider"]
version = request["version"]
capability = request["capability"]
if capability == "classify":
    proposals = []
    for process in request["processes"]:
        cwd = process.get("cwd")
        if cwd and (Path(cwd) / ".reswho-example").is_file():
            proposals.append({
                "identity": {"pid": process["pid"], "started_unix_us": process["started_unix_us"]},
                "evidence": {"kind": "custom", "key": "marked-workspace", "label": "Example workspace", "path": cwd, "basis": "cwd"},
            })
    contribution = {"capability": "classify", "proposals": proposals}
elif capability == "details":
    process = request["process"]
    contribution = {
        "capability": "parsed_details",
        "identity": {"pid": process["pid"], "started_unix_us": process["started_unix_us"]},
        "fields": [
            {"key": f"extensions.{provider}.sample", "label": "Shared sample ms", "value": request["settings"]["sampling.interval_ms"], "sensitive": False},
            {"key": f"extensions.{provider}.host", "label": "Host CLI version", "value": request["host"]["version"], "sensitive": False},
            {"key": f"extensions.{provider}.token", "label": "Example token", "value": "fixture-secret-value", "sensitive": False},
        ],
    }
else:
    raise SystemExit(2)
json.dump({"protocol_version": 1, "provider": provider, "version": version, "contribution": contribution}, sys.stdout)
