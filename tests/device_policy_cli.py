#!/usr/bin/env python3
"""Device access posture and expected-down policies stay local and bounded."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile

binary = str(pathlib.Path(sys.argv[1]).resolve()) if len(sys.argv) > 1 else str(pathlib.Path(__file__).resolve().parents[1] / "target/debug/cx")
with tempfile.TemporaryDirectory(prefix="cx-device-policy-") as tmp:
    state = pathlib.Path(tmp) / "cx"
    state.mkdir(mode=0o700)
    (state / "devices.json").write_text(json.dumps([
        dict(id="local", name="innovation", target=None, account="tester", host="innovation", status="local", observed_at=0),
        dict(id="creation-id", name="creation", target="conception@example", account="conception", host="creation", status="unknown", observed_at=0),
        dict(id="offline-id", name="tranquility", target="peace@example", account="peace", host="tranquility", status="unknown", observed_at=0),
    ]))
    env = dict(os.environ, XDG_STATE_HOME=tmp)

    def run(*args):
        return subprocess.run([binary, *args], env=env, capture_output=True, text=True, check=True)

    promoted = json.loads(run("device", "promote", "creation").stdout)
    assert promoted["access"] == "core" and promoted["bidirectional"]
    run("device", "availability", "tranquility", "down")
    report = json.loads(run("device", "policy", "tranquility").stdout)
    assert report["availability"] == "usually_down"
    run("device", "demote", "creation")
    report = json.loads(run("device", "policy", "creation").stdout)
    assert report["access"] == "directed"
    saved = json.loads((state / "device-policies.json").read_text())
    assert saved["devices"]["offline-id"]["availability"] == "usually_down"
    assert saved["devices"]["creation-id"]["access"] == "directed"
print(json.dumps(dict(result="PASS", promotion_and_demotion=True, expected_down=True, owner_checked_store=True)))
