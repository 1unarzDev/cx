#!/usr/bin/env python3
"""The mesh util offline (mesh/mesh.py, docs/mesh.md): profile excludes, the config template, the device scripts'
syntax and placeholders, and a dry-run join that changes nothing. No Cloudflare account is touched."""
import io
import ipaddress
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "mesh"))
import mesh  # noqa: E402

W = ROOT / "mesh" / "worker"


class MeshUtil(unittest.TestCase):
    def test_mesh_is_carved_out_of_the_excludes(self):
        ex = [ipaddress.ip_network(e["address"]) for e in mesh.excludes(mesh.DEFAULT_PROFILE)]
        mesh_net = ipaddress.ip_network("100.96.0.0/12")
        self.assertFalse(any(n.version == 4 and n.overlaps(mesh_net) for n in ex))
        self.assertIn(ipaddress.ip_network("100.64.0.0/11"), ex)
        self.assertIn(ipaddress.ip_network("192.168.0.0/16"), ex)

    def test_tunneled_lans_ride_the_mesh(self):
        p = {**mesh.DEFAULT_PROFILE, "tunneled_lans": ["192.168.1.0/24"]}
        ex = [ipaddress.ip_network(e["address"]) for e in mesh.excludes(p)]
        self.assertFalse(any(n.overlaps(ipaddress.ip_network("192.168.1.0/24")) for n in ex if n.version == 4))
        self.assertIn(ipaddress.ip_network("192.168.0.0/24"), ex)

    def test_config_round_trip(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "config.toml"
            mesh.write_config(p, "demo", "acct", "")
            c = mesh.load_config(str(p))
            self.assertEqual((c["name"], c["dns_suffix"], c["profile"]["tunnel_protocol"]), ("demo", ".mesh", "masque"))
            mesh.write_config(p, "demo", "acct", "example.org")
            self.assertEqual(mesh.load_config(str(p))["dns_suffix"], ".mesh.example.org")

    def test_scripts_parse_and_carry_no_instance(self):
        for f in ("join.sh", "leave.sh"):
            subprocess.run(["sh", "-n", str(W / f)], check=True)
        for f in ("join.sh", "leave.sh", "join.ps1", "leave.ps1", "worker.js"):
            t = (W / f).read_text()
            self.assertNotIn("lunarz", t.lower(), f)
            self.assertNotRegex(t, r"\b100\.96\.\d+\.\d+\b", f)

    def test_dry_run_join_changes_nothing(self):
        body = (W / "join.sh").read_text().replace("__MESH_URL__", "https://mesh.example.org").replace("__MESH_CODE__", "abcd2345")
        env = {**os.environ, "MESH_DRY_RUN": "1", "MESH_NAME": "probe"}
        with tempfile.TemporaryDirectory() as d:
            fake = Path(d) / "warp-cli"
            fake.write_text("#!/bin/sh\nexit 1\n")
            fake.chmod(0o755)
            sudo = Path(d) / "sudo"
            sudo.write_text('#!/bin/sh\nexec "$@"\n')
            sudo.chmod(0o755)
            env["PATH"] = f"{d}:{env['PATH']}"
            r = subprocess.run(["sh", "-s"], input=body, text=True, capture_output=True, env=env)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("would write /var/lib/cloudflare-warp/mdm.xml", r.stderr)
        self.assertIn("ssh ", r.stdout)


class FakeCloudflare:
    """Just enough of the Cloudflare API for setup and teardown, with the resources it holds."""

    def __init__(self):
        self.res = {"policies": [], "tokens": [], "access_policies": [], "apps": [{"id": "app", "type": "warp", "name": "Warp Login App", "policies": [{"id": "keep"}]}], "d1": [], "scripts": {}}
        self.calls, self.n = [], 0

    def __call__(self, method, url, body=None, headers=None, raw=None, ctype=None):
        if "/client/v4" not in url:  # the deployed Worker
            self.calls.append(f"{method} worker{url.split('.dev', 1)[1]}")
            return {"enrolled": [], "connectors": [], "dns": []}
        path = url.split("/client/v4")[1].split("?")[0]
        self.calls.append(f"{method} {path}")
        r, ok = self.res, lambda x: {"success": True, "result": x}
        self.n += 1
        nid = f"id{self.n}"
        if path == "/accounts":
            return ok([{"id": "acct"}])
        p = path.removeprefix("/accounts/acct")
        if p == "/access/organizations":
            return ok({"auth_domain": "team.cloudflareaccess.com"})
        if p == "/devices/settings":
            return ok({"use_zt_virtual_ip": False})
        if p == "/devices/policies":
            return ok(r["policies"])
        if p == "/devices/policy" and method == "POST":
            r["policies"].append({**body, "policy_id": nid})
            return ok(r["policies"][-1])
        if p.startswith("/devices/policy/") and method == "DELETE":
            r["policies"] = [x for x in r["policies"] if x["policy_id"] != p.split("/")[3]]
        if p == "/access/service_tokens" and method == "GET":
            return ok(r["tokens"])
        if p == "/access/service_tokens":
            r["tokens"].append({**body, "id": nid})
            return ok(r["tokens"][-1])
        if p.startswith("/access/service_tokens/") and method == "DELETE":
            r["tokens"] = [x for x in r["tokens"] if x["id"] != p.split("/")[3]]
        if p == "/access/policies" and method == "GET":
            return ok(r["access_policies"])
        if p == "/access/policies":
            r["access_policies"].append({**body, "id": nid})
            return ok(r["access_policies"][-1])
        if p.startswith("/access/policies/") and method == "DELETE":
            r["access_policies"] = [x for x in r["access_policies"] if x["id"] != p.split("/")[3]]
        if p == "/access/apps":
            return ok(r["apps"])
        if p == "/access/apps/app" and method == "GET":
            return ok(r["apps"][0])
        if p == "/access/apps/app" and method == "PUT":
            r["apps"][0]["policies"] = body["policies"]
        if p == "/d1/database" and method == "GET":
            return ok(r["d1"])
        if p == "/d1/database" and method == "POST":
            r["d1"].append({**body, "uuid": nid})
            return ok(r["d1"][-1])
        if p.startswith("/d1/database/") and method == "DELETE":
            r["d1"] = []
        if p == "/workers/subdomain":
            return ok({"subdomain": "owner"})
        if p == "/tokens/permission_groups":
            return ok([{"id": n, "name": n, "scopes": ["com.cloudflare.api.account"]} for n in mesh.WORKER_SCOPES])
        if p == "/tokens" and method == "POST":
            return ok({"id": "wtok", "value": "worker-secret"})
        if p.startswith("/workers/scripts/") and method == "PUT" and "/subdomain" not in p:
            r["scripts"][p.split("/")[3]] = raw
        if p.startswith("/workers/scripts/") and method == "DELETE":
            r["scripts"].pop(p.split("/")[3], None)
        return ok({})


class MeshSetup(unittest.TestCase):
    def run_mesh(self, d, fake, **kw):
        m = mesh.Mesh(mesh.load_config(str(Path(d) / "config.toml")), token="t", out=io.StringIO(), **kw)
        m.http = fake
        return m

    def test_setup_is_idempotent_and_teardown_removes_what_it_made(self):
        fake = FakeCloudflare()
        with tempfile.TemporaryDirectory() as d:
            mesh.write_config(Path(d) / "config.toml", "demo", "", "")
            st = self.run_mesh(d, fake).setup()
            self.assertEqual(st["url"], "https://demo-mesh.owner.workers.dev")
            prof = fake.res["policies"][0]
            self.assertEqual((prof["name"], prof["tunnel_protocol"]), ("demo-mesh", "masque"))
            self.assertEqual(prof["match"], 'identity.email == "non_identity@team.cloudflareaccess.com"')
            self.assertEqual([p["id"] for p in fake.res["apps"][0]["policies"]], ["keep", st["policy_id"]])
            script = fake.res["scripts"]["demo-mesh"]
            for part in (b'name="worker.js"', b'name="join.sh"', b'name="config.json"', b'"CF_API_TOKEN"', b"worker-secret"):
                self.assertIn(part, script)
            self.assertNotIn("worker-secret", (Path(d) / "demo.state.json").read_text())
            first = len(fake.calls)
            self.run_mesh(d, fake).setup()
            creates = [c for c in fake.calls[first:] if c.startswith("POST") and not c.endswith(("/query", "/subdomain"))]
            self.assertEqual(creates, [], "a second setup creates nothing")
            self.assertEqual(len(fake.res["policies"]), 1)
            self.assertNotIn(b"worker-secret", fake.res["scripts"]["demo-mesh"], "the secret binding is kept, not resent")
            self.run_mesh(d, fake).teardown()
            self.assertEqual((fake.res["policies"], fake.res["tokens"], fake.res["access_policies"], fake.res["d1"], fake.res["scripts"]), ([], [], [], [], {}))
            self.assertEqual([p["id"] for p in fake.res["apps"][0]["policies"]], ["keep"], "the login app keeps its other policies")

    def test_dry_run_changes_nothing(self):
        fake = FakeCloudflare()
        with tempfile.TemporaryDirectory() as d:
            mesh.write_config(Path(d) / "config.toml", "demo", "acct", "")
            st = self.run_mesh(d, fake, dry_run=True).setup()
            self.assertTrue(st["planned"])
            self.assertFalse([c for c in fake.calls if not c.startswith("GET")])
            self.assertFalse((Path(d) / "demo.state.json").exists())

    def test_json_after_the_subcommand(self):
        with tempfile.TemporaryDirectory() as d:
            r = subprocess.run([sys.executable, str(ROOT / "mesh" / "mesh.py"), "devices", "--json", "--config", f"{d}/none.toml"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        self.assertIn("no mesh config", json.loads(r.stdout)["error"])


if __name__ == "__main__":
    unittest.main()
