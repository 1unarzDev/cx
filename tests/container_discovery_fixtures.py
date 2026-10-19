#!/usr/bin/env python3
"""Hermetic discovery regression: fake Docker, private state, no live containers."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix="cx-discovery-") as tmp:
    root = Path(tmp)
    workspace = root / "project"
    (workspace / ".devcontainer").mkdir(parents=True)
    (workspace / ".devcontainer/devcontainer.json").write_text('{"remoteUser":"wrong"}')
    custom = workspace / ".devcontainer/custom.json"
    custom.write_text('{"remoteUser":"developer","workspaceFolder":"/project"}')
    ids = ["a" * 64, "b" * 64, "c" * 64]
    fixture = root / "fixture.json"
    fixture.write_text(json.dumps({
        "ids": ids,
        "containers": {
            ids[0]: dict(id=ids[0], name="/running-dev", state="running", user="root",
                         workspace=str(workspace), config=str(custom)),
            ids[1]: dict(id=ids[1], name="/stopped-dev", state="exited", user="root",
                         workspace="/no-longer-mounted-project"),
        },
    }))
    docker = root / "docker"
    docker.write_text('''#!/usr/bin/env python3
import json, os, sys
f=json.load(open(os.environ["CX_DOCKER_FIXTURE"]))
a=sys.argv[1:]
if a[0]=="ps": print("\\n".join(f["ids"]))
elif a[0]=="context": print("unix:///fixture/docker.sock")
elif a[0]=="info": print("fixture-engine")
elif a[0]=="inspect":
 v=f["containers"].get(a[-1])
 if v is None: sys.exit(1)
 print(json.dumps(v))
else: sys.exit(99)
''')
    docker.chmod(0o700)
    env = dict(os.environ, PATH=tmp + os.pathsep + os.environ["PATH"],
               HOME=tmp, XDG_STATE_HOME=tmp + "/state", CX_DOCKER_FIXTURE=str(fixture))
    env.pop("DOCKER_CONTEXT", None)
    env.pop("DOCKER_HOST", None)
    request = json.dumps(dict(version=1, id="discovery", op=dict(op="containers"))).encode()
    def discover():
        output = subprocess.check_output([binary, "helper"], env=env,
            input=b"CX1 " + str(len(request)).encode() + b"\n" + request, timeout=20)
        reply = json.loads(output.split(b"\n", 1)[1])
        assert not reply.get("error"), reply.get("error")
        return reply["result"]
    result = discover()
    assert result["truncated"], "failed inspection was presented as a complete discovery"
    assert result["complete"] is False and result["failed_inspections"] == 1
    entries = {c["id"]: c for c in result["containers"]}
    assert entries[ids[0]]["user"] == "developer", "explicit devcontainer configuration was ignored"
    assert entries[ids[0]]["folder"] == "/project"
    assert entries[ids[1]]["workspace"] == "/no-longer-mounted-project", "workspace label was lost after reconnect"
    assert entries[ids[1]]["state"] == "exited", "real stopped containers must remain available for explicit Start"
    data = json.loads(fixture.read_text())
    data["ids"] = ids[:2]
    fixture.write_text(json.dumps(data))
    result = discover()
    assert result["complete"] and not result["truncated"] and result["failed_inspections"] == 0
    print("PASS: incomplete discovery, explicit config, workspace label recovery, stopped-state evidence, complete rescan")
