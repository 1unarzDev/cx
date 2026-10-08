#!/usr/bin/env python3
"""Validate scoped existing-LAN nft plans only in a new user/network namespace."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

binary = str(pathlib.Path(sys.argv[1]).resolve())
if os.environ.get("CX_LAN_NAMESPACE") != "1":
    available = subprocess.run(["unshare", "-Urn", "true"], capture_output=True)
    if available.returncode:
        raise SystemExit("BLOCKED: user/network namespaces unavailable")
    env = dict(os.environ, CX_LAN_NAMESPACE="1")
    os.execvpe("unshare", ["unshare", "-Urn", sys.executable, __file__, binary], env)

def run(*args, text=None):
    return subprocess.run(args, input=text, text=True, capture_output=True,
                          timeout=10, check=True).stdout

with tempfile.TemporaryDirectory(prefix="cx-preserve-lan-") as temporary:
    path = pathlib.Path(temporary) / "observation.json"
    path.write_text(json.dumps(dict(observed_at=int(time.time()), downstream="robotlan",
        upstream="uplink", gateway="192.168.0.2", forwarding_enabled=True,
        mesh_addresses=["100.96.0.12", "100.96.0.27", "100.96.0.28"],
        recipients=[dict(id="blastoise", address="192.168.0.147", gateway="192.168.0.2", dns=["1.1.1.1"])])))
    for interface in ("robotlan", "uplink"):
        run("ip", "link", "add", interface, "type", "dummy")
        run("ip", "link", "set", interface, "up")
    run("ip", "address", "add", "192.168.0.2/24", "dev", "robotlan")
    run("ip", "address", "add", "198.18.0.2/24", "dev", "uplink")
    run("ip", "route", "add", "default", "via", "198.18.0.1", "dev", "uplink")
    before = run("ip", "-j", "address"), run("ip", "-j", "route")
    run("nft", "-f", "-", text="table inet unrelated {\n chain preserve {\n }\n}\n")
    baseline = run("nft", "-j", "list", "table", "inet", "unrelated")
    plan = json.loads(run(binary, "share-plan", "robot_lan", "--observation", str(path)))
    assert not plan["ready_for_apply"]
    assert plan["required_gateway_changes"] == [] and plan["required_dns_configuration"] == []
    run("nft", "-c", "-f", "-", text=plan["nft_rules"])
    run("nft", "-f", "-", text=plan["nft_rules"])
    rules = run("nft", "list", "table", "inet", "cx_robot_lan")
    assert "ct state new reject with tcp reset" in rules
    assert "ct state established,related" in rules
    assert "masquerade" in run("nft", "list", "table", "ip", "cx_robot_lan_nat")
    assert before == (run("ip", "-j", "address"), run("ip", "-j", "route"))
    assert baseline == run("nft", "-j", "list", "table", "inet", "unrelated")
    run("nft", "delete", "table", "ip", "cx_robot_lan_nat")
    run("nft", "delete", "table", "inet", "cx_robot_lan")
    assert before == (run("ip", "-j", "address"), run("ip", "-j", "route"))
    assert baseline == run("nft", "-j", "list", "table", "inet", "unrelated")
print(json.dumps(dict(result="PASS", isolated_namespace=True, actual_nft_syntax_and_lifecycle=True,
                     preserved_addresses_routes_unrelated_firewall=True,
                     robot_dns_https_and_reverse_denial="NOT TESTED")))
