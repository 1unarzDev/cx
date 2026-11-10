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
import unittest.mock
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "mesh"))
import mesh  # noqa: E402
import claude  # noqa: E402

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
        for f in ("join.sh", "leave.sh", "claude.sh"):
            subprocess.run(["sh", "-n", str(W / f)], check=True)
        for f in ("join.sh", "leave.sh", "join.ps1", "leave.ps1", "claude.sh", "claude.ps1", "worker.js", "../claude.py"):
            t = (W / f).read_text()
            self.assertNotIn("lunarz", t.lower(), f)
            self.assertNotRegex(t, r"\b100\.96\.\d+\.\d+\b", f)

    def test_device_script_never_sees_the_api_token(self):
        calls = []

        def run(argv, **kw):
            calls.append((argv, kw.get("env")))
            return subprocess.CompletedProcess(argv, 0, b"true\n", b"")
        with unittest.mock.patch.dict(os.environ, {"CLOUDFLARE_API_TOKEN": "cf-admin-canary"}), \
                unittest.mock.patch.object(mesh.subprocess, "run", run):
            self.assertEqual(mesh.device_script("https://mesh.example.org/abc", False), 0)
        self.assertEqual([argv[0] for argv, _ in calls], ["curl", "sh"])
        for _, env in calls:
            self.assertIsNotNone(env)
            self.assertNotIn("CLOUDFLARE_API_TOKEN", env)
            self.assertNotIn("cf-admin-canary", repr(env))

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


class FakeLendWorld:
    """Cloudflare (the Access app, its policies, service tokens), the mesh Worker and the better-ccflare CLI."""

    def __init__(self):
        self.tokens, self.app_pols, self.keys, self.lends, self.n = [], [{"id": "keep", "name": "people", "precedence": 1}], {}, {}, 0

    def http(self, method, url, body=None, headers=None, raw=None, ctype=None):
        ok = lambda x: {"success": True, "result": x}  # noqa: E731
        self.n += 1
        if url.startswith("https://mesh.example.org"):
            p = url.split(".org", 1)[1]
            if p == "/v1/admin/claude" and method == "POST":
                self.lends[body["name"]] = {**body, "state": "pending"}
                return {"name": body["name"], "code": "c0de", "code_expires": "2030-01-01T00:00:00Z", "sh": "curl …|sh", "ps1": "irm …|iex"}
            if p == "/v1/admin/claude":
                return {"lends": [{"name": k, "state": v["state"], "expires": v["expires"]} for k, v in self.lends.items()]}
            name = p.split("/")[4]
            hit = name in self.lends and self.lends[name]["state"] != "revoked"
            if hit:
                self.lends[name]["state"] = "revoked"
            return {"name": name, "revoked": hit}
        p = url.split("/accounts/acct")[1].split("?")[0]
        if p == "/access/apps":
            return ok([{"id": "app", "domain": "claude.example.org", "type": "self_hosted"}])
        if p == "/access/apps/app/policies" and method == "GET":
            return ok(self.app_pols)
        if p == "/access/apps/app/policies":
            self.app_pols.append({**body, "id": f"pol{self.n}"})
            return ok(self.app_pols[-1])
        if p.startswith("/access/apps/app/policies/") and method == "DELETE":
            self.app_pols = [x for x in self.app_pols if x["id"] != p.split("/")[-1]]
            return ok({})
        if p == "/access/service_tokens" and method == "GET":
            return ok(self.tokens)
        if p == "/access/service_tokens":
            self.tokens.append({**body, "id": f"tok{self.n}", "client_id": "cid.access", "client_secret": "csecret"})
            return ok(self.tokens[-1])
        if p.startswith("/access/service_tokens/") and method == "DELETE":
            assert not any(x.get("include", [{}])[0].get("service_token", {}).get("token_id") == p.split("/")[-1] for x in self.app_pols), "token still named"
            self.tokens = [x for x in self.tokens if x["id"] != p.split("/")[-1]]
            return ok({})
        raise AssertionError(f"unexpected {method} {p}")

    def gateway(self, argv, **kw):
        op, *rest = argv[3:]  # after ssh gw bcf
        out = "\x1b[1m◇ injecting env\x1b[0m\n"
        if op == "--list-api-keys":
            out += "API Keys:\n" + "".join(f"  {k} (Pfx12345)\n    Status: {v}\n    Role: API-only\n" for k, v in self.keys.items())
        elif op == "--generate-api-key":
            if rest[0] in self.keys:
                out += f"❌ Failed to generate API key: API key with name '{rest[0]}' already exists\n"
            else:
                self.keys[rest[0]] = "Active"
                out += f"API Key Generated Successfully!\n\nName: {rest[0]}\nKey: btr-{rest[0]}-secret  Save this key now\n"
        elif op == "--disable-api-key":
            self.keys[rest[0]] = "Disabled"
        elif op == "--delete-api-key":
            self.keys.pop(rest[0])
        return subprocess.CompletedProcess(argv, 0, out, "")


class ClaudeLend(unittest.TestCase):
    def lends(self, d, world, extra=""):
        cfg = Path(d) / "config.toml"
        mesh.write_config(cfg, "demo", "acct", "example.org")
        cfg.write_text(cfg.read_text().replace("[devices]", f'[claude]\ngateway = ["ssh", "gw", "bcf"]\nbase_url = "https://claude.example.org"\n{extra}\n[devices]'))
        (Path(d) / "demo.state.json").write_text(json.dumps({"url": "https://mesh.example.org"}))
        m = mesh.Mesh(mesh.load_config(str(cfg)), token="t", out=io.StringIO())
        m.http = world.http
        return claude.Lends(m, run=world.gateway)

    def test_lend_then_revoke_leaves_only_a_disabled_key(self):
        w = FakeLendWorld()
        with tempfile.TemporaryDirectory() as d:
            L = self.lends(d, w)
            r = L.lend("evan", hours=48)
            self.assertEqual((r["code"], r["route"]), ("c0de", "access"))
            self.assertEqual(w.keys, {"cc-lend-evan": "Active"})
            self.assertEqual([(t["name"], t["duration"]) for t in w.tokens], [("cc-lend-evan", "48h")])
            pol = w.app_pols[-1]
            self.assertEqual((pol["name"], pol["decision"], pol["precedence"]), ("cc-lend-evan", "non_identity", 2))
            sent = w.lends["evan"]["bundle"]
            self.assertEqual(sent, {"base_url": "https://claude.example.org", "token": "btr-cc-lend-evan-secret",
                                    "headers": ["CF-Access-Client-Id: cid.access", "CF-Access-Client-Secret: csecret"]})
            self.assertNotIn("secret", L.m.out.getvalue(), "no secret in the log")
            again = L.lend("evan")  # replaces: one token, one policy, a fresh key
            self.assertEqual(again["code"], "c0de")
            self.assertEqual((len(w.tokens), len(w.app_pols)), (1, 2))
            self.assertEqual(L.listing()["evan"]["key"], "Active")
            self.assertEqual(L.revoke("evan")["revoked"], ["policy", "service token", "gateway key", "code"])
            self.assertEqual((w.tokens, [p["id"] for p in w.app_pols], w.keys), ([], ["keep"], {"cc-lend-evan": "Disabled"}))
            self.assertEqual(L.revoke("evan")["revoked"], [], "a second revoke finds nothing")

    def test_gateway_command_never_sees_the_api_token(self):
        w = FakeLendWorld()
        envs = []
        real = w.gateway
        w.gateway = lambda argv, **kw: (envs.append(kw.get("env")), real(argv, **kw))[1]
        with tempfile.TemporaryDirectory() as d, unittest.mock.patch.dict(os.environ, {"CLOUDFLARE_API_TOKEN": "cf-admin-canary"}):
            self.lends(d, w).lend("evan", hours=1)
        self.assertTrue(envs and all(e is not None and "CLOUDFLARE_API_TOKEN" not in e for e in envs))
        self.assertEqual(envs[0].get("PATH"), os.environ.get("PATH"))

    def test_mesh_only_lend_makes_no_access_token(self):
        w = FakeLendWorld()
        with tempfile.TemporaryDirectory() as d:
            L = self.lends(d, w, 'mesh_url = "http://100.96.0.5:8081"')
            self.assertEqual(L.lend("nick", mesh_only=True)["route"], "mesh")
            self.assertEqual((w.tokens, w.lends["nick"]["bundle"]["base_url"], w.lends["nick"]["mesh_url"]), ([], "", "http://100.96.0.5:8081"))

    def test_a_failed_lend_undoes_itself(self):
        w = FakeLendWorld()
        real = w.http
        w.http = lambda method, url, body=None, **kw: (_ for _ in ()).throw(mesh.MeshError("worker down")) if method == "POST" and url.endswith("/v1/admin/claude") else real(method, url, body, **kw)
        with tempfile.TemporaryDirectory() as d:
            L = self.lends(d, w)
            with self.assertRaises(mesh.MeshError):
                L.lend("evan")
            self.assertEqual((w.tokens, [p["id"] for p in w.app_pols], w.keys), ([], ["keep"], {"cc-lend-evan": "Disabled"}))

    def test_expired_lends_are_revoked(self):
        w = FakeLendWorld()
        with tempfile.TemporaryDirectory() as d:
            L = self.lends(d, w)
            L.lend("old", hours=1)
            L.lend("new")
            w.lends["old"]["expires"] = "2020-01-01T00:00:00+00:00"
            self.assertEqual([r["name"] for r in L.expired()], ["old"])
            self.assertEqual(w.keys, {"cc-lend-old": "Disabled", "cc-lend-new": "Active"})


class ClaudeScript(unittest.TestCase):
    """claude.sh against a local stand-in for the Worker and the gateway: merge, backup, verify, no secret shown."""

    def serve(self, health=200):
        import http.server
        import threading

        seen = self.seen = []

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def reply(self, code, body=b"", ctype="text/plain"):
                self.send_response(code)
                self.send_header("content-type", ctype)
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                self.reply(health if self.path == "/health" else 404)

            def do_POST(self):
                body = self.rfile.read(int(self.headers.get("content-length", 0)))
                seen.append((self.path, dict(self.headers), body))
                if self.path == "/v1/claude/claim":
                    if json.loads(body)["code"] != "goodcode2345" or sum(p == "/v1/claude/claim" for p, *_ in seen) > 1:
                        return self.reply(403, b'{"error":"this code is unknown, already used or expired"}', "application/json")
                    return self.reply(200, f"name=evan\nbase_url={url}\nmesh_url=\ntoken=btr-sekrit\nheader=CF-Access-Client-Id: cid.access\nheader=CF-Access-Client-Secret: csecret\n".encode())
                if self.path == "/v1/messages/count_tokens":
                    ok = self.headers.get("authorization") == "Bearer btr-sekrit"
                    return self.reply(200 if ok else 401, b'{"input_tokens":8}', "application/json")
                self.reply(404)

        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        url = f"http://127.0.0.1:{srv.server_address[1]}"
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.shutdown)
        return url

    def run_script(self, url, d, mesh_url="", mesh_only="", code="goodcode2345"):
        body = (W / "claude.sh").read_text()
        for k, v in {"__MESH_URL__": url, "__CLAUDE_CODE__": code, "__CLAUDE_MESH_URL__": mesh_url, "__CLAUDE_MESH_ONLY__": mesh_only}.items():
            body = body.replace(k, v)
        env = {k: v for k, v in os.environ.items() if not k.startswith("ANTHROPIC_")}
        return subprocess.run(["sh", "-s"], input=body, text=True, capture_output=True, env={**env, "CLAUDE_CONFIG_DIR": d}, timeout=60)

    def test_access_route_merges_backs_up_and_verifies(self):
        url = self.serve()
        with tempfile.TemporaryDirectory() as d:
            s = Path(d) / "settings.json"
            s.write_text(json.dumps({"model": "opus", "env": {"KEEP": "1"}}))
            r = self.run_script(url, d)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("ready", r.stderr)
            for secret in ("btr-sekrit", "csecret"):
                self.assertNotIn(secret, r.stdout + r.stderr)
            got = json.loads(s.read_text())
            self.assertEqual(got["model"], "opus")
            self.assertEqual(got["env"], {"KEEP": "1", "ANTHROPIC_BASE_URL": url, "ANTHROPIC_AUTH_TOKEN": "btr-sekrit",
                                          "ANTHROPIC_CUSTOM_HEADERS": "CF-Access-Client-Id: cid.access\nCF-Access-Client-Secret: csecret"})
            self.assertEqual(s.stat().st_mode & 0o777, 0o600)
            self.assertEqual([json.loads(b.read_text()) for b in Path(d).glob("settings.json.bak-*")], [{"model": "opus", "env": {"KEEP": "1"}}])
            verify = [h for p, h, _ in self.seen if p == "/v1/messages/count_tokens"][0]
            self.assertEqual(verify.get("CF-Access-Client-Secret"), "csecret")
            again = self.run_script(url, d)
            self.assertNotEqual(again.returncode, 0)
            self.assertIn("already used", again.stderr)

    def test_mesh_route_drops_the_access_headers(self):
        url = self.serve()
        with tempfile.TemporaryDirectory() as d:
            r = self.run_script(url, d, mesh_url=url)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("route: mesh", r.stderr)
            env = json.loads((Path(d) / "settings.json").read_text())["env"]
            self.assertNotIn("ANTHROPIC_CUSTOM_HEADERS", env)

    def test_mesh_only_off_the_mesh_keeps_the_code(self):
        url = self.serve()
        with tempfile.TemporaryDirectory() as d:
            r = self.run_script(url, d, mesh_url="http://127.0.0.1:9", mesh_only="1")
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("still unused", r.stderr)
            self.assertEqual(self.seen, [], "nothing claimed")


class Onboarding(unittest.TestCase):
    """token-url, verify and zones: the onboarding helpers of mesh.py --json."""

    def fake_api(self, routes):
        seen = []

        def request(method, url, token=None, body=None, timeout=30):
            seen.append((method, url.split("/client/v4")[-1], token, body))
            for prefix, reply in routes:
                if url.split("/client/v4")[-1].startswith(prefix):
                    return reply(token) if callable(reply) else reply
            return {"success": False, "status": 404, "errors": [{"message": "no route"}]}

        return request, seen

    def test_token_url_prefills_the_documented_permissions(self):
        import urllib.parse as up
        d = mesh.token_url()
        u = up.urlparse(d["url"])
        self.assertEqual((u.scheme, u.netloc, u.path), ("https", "dash.cloudflare.com", "/profile/api-tokens"))
        q = up.parse_qs(u.query)
        self.assertEqual((q["accountId"], q["zoneId"], q["name"]), (["*"], ["all"], ["cx-mesh"]))
        keys = json.loads(q["permissionGroupKeys"][0])
        self.assertEqual([k["key"] for k in keys], ["teams", "access", "access_acct", "d1", "workers_scripts", "account_api_tokens"])
        self.assertTrue(all(k["type"] in ("read", "edit") for k in keys), "the template takes edit, not write")
        self.assertNotIn(" ", d["url"])
        self.assertEqual(d["manual"], ["Access: Service Tokens Write", "Cloudflare One Connector: WARP Write"])
        docs = (ROOT / "docs" / "mesh.md").read_text()
        for name in d["permissions"] + d["manual"] + ["Workers Routes Write", "Zone Read"]:
            self.assertIn(name.split(", Identity")[0], docs, f"docs/mesh.md names {name}")
        wd = json.loads(up.parse_qs(up.urlparse(mesh.token_url(True)["url"]).query)["permissionGroupKeys"][0])
        self.assertEqual(wd[-2:], [{"key": "workers_routes", "type": "edit"}, {"key": "zone", "type": "read"}])
        r = subprocess.run([sys.executable, str(ROOT / "mesh" / "mesh.py"), "token-url", "--json"], capture_output=True, text=True, env={**os.environ, "CLOUDFLARE_API_TOKEN": ""})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout)["url"], d["url"])

    def test_verify_user_then_account_and_never_echoes_the_token(self):
        req, seen = self.fake_api([
            ("/user/tokens/verify", {"success": False, "status": 401, "errors": [{"message": "Invalid API Token"}]}),
            ("/accounts/acct/tokens/verify", lambda t: {"success": True, "result": {"id": "tid", "status": "active" if t == "sekrit-acct" else "disabled"}}),
        ])
        old, mesh.request = mesh.request, req
        try:
            d = mesh.verify_token("sekrit-acct", "acct")
            self.assertEqual((d["valid"], d["status"], d["kind"], d["id"]), (True, "active", "account", "tid"))
            self.assertNotIn("sekrit", json.dumps(d))
            self.assertEqual([x[1] for x in seen], ["/user/tokens/verify", "/accounts/acct/tokens/verify"])
            d = mesh.verify_token("other", None)
            self.assertEqual((d["valid"], d["status"]), (False, "invalid"))
            self.assertEqual(d["errors"], ["Invalid API Token"])
            mesh.request = self.fake_api([("/user/tokens/verify", {"success": True, "result": {"id": "u", "status": "active", "expires_on": "2027-01-01T00:00:00Z"}})])[0]
            d = mesh.verify_token("t")
            self.assertEqual((d["valid"], d["kind"], d["expires_on"]), (True, "user", "2027-01-01T00:00:00Z"))
            with self.assertRaises(mesh.MeshError):
                mesh.verify_token("")
        finally:
            mesh.request = old

    def test_zones_for_the_domain_choice(self):
        req, seen = self.fake_api([("/zones", {"success": True, "result": [{"id": "z1", "name": "example.org", "status": "active", "account": {"id": "acct", "name": "Me"}, "name_servers": ["x"]}], "result_info": {"total_pages": 1}})])
        old, mesh.request = mesh.request, req
        try:
            d = mesh.list_zones("t")
            self.assertEqual(d, {"zones": [{"id": "z1", "name": "example.org", "status": "active", "account": {"id": "acct", "name": "Me"}}], "more": False})
            self.assertEqual(seen[0][:3], ("GET", "/zones?per_page=50", "t"))
            mesh.request = self.fake_api([("/zones", {"success": False, "status": 403, "errors": ["denied"]})])[0]
            with self.assertRaises(mesh.MeshError):
                mesh.list_zones("t")
        finally:
            mesh.request = old

    def test_verify_cli_reads_the_token_from_stdin(self):
        # A stub module replaces urllib's network: the CLI path, argument parsing and exit status.
        with tempfile.TemporaryDirectory() as d:
            stub = Path(d) / "run.py"
            stub.write_text(
                "import json,sys\nsys.path.insert(0, %r)\nimport mesh\n"
                "mesh.request = lambda m, u, t=None, b=None, timeout=30: {'success': True, 'result': {'id': 'x', 'status': 'active' if t == 'good' else 'expired'}}\n"
                "sys.exit(mesh.main(sys.argv[1:]))\n" % str(ROOT / "mesh"))
            env = {**os.environ, "CLOUDFLARE_API_TOKEN": "", "XDG_CONFIG_HOME": d}
            r = subprocess.run([sys.executable, str(stub), "verify", "--token-stdin", "--json"], input="good\n", capture_output=True, text=True, env=env)
            self.assertEqual((r.returncode, json.loads(r.stdout)["valid"]), (0, True), r.stderr)
            self.assertNotIn("good", r.stdout.replace('"valid": true', ""))
            r = subprocess.run([sys.executable, str(stub), "verify", "--token-stdin", "--json"], input="bad\n", capture_output=True, text=True, env=env)
            self.assertEqual((r.returncode, json.loads(r.stdout)["status"]), (1, "expired"))

class Teardown(unittest.TestCase):
    def test_teardown_dry_run_lists_without_calls(self):
        fake = FakeCloudflare()
        with tempfile.TemporaryDirectory() as d:
            mesh.write_config(Path(d) / "config.toml", "demo", "", "")
            MeshSetup.run_mesh(None, d, fake).setup()
            before = len(fake.calls)
            r = MeshSetup.run_mesh(None, d, fake).teardown(dry_run=True)
            self.assertEqual(len(fake.calls), before, "a dry run calls nothing")
            self.assertIn("worker", r["would_remove"])
            self.assertTrue((Path(d) / "demo.state.json").exists())

    def test_teardown_cli_dry_run_wins_over_yes(self):
        # cx's cloudflare-warp backend runs `teardown --yes --dry-run --json`: a preview, never a teardown.
        with tempfile.TemporaryDirectory() as d:
            mesh.write_config(Path(d) / "config.toml", "demo", "acct", "")
            state = {"url": "https://demo-mesh.owner.workers.dev", "worker": "demo-mesh", "d1_id": "db"}
            (Path(d) / "demo.state.json").write_text(json.dumps(state))
            stub = Path(d) / "run.py"
            stub.write_text(
                "import sys\nsys.path.insert(0, %r)\nimport mesh\n"
                "def refuse(*a, **k):\n    raise SystemExit('network call in a dry run')\n"
                "mesh.Mesh.http = refuse\nmesh.request = refuse\n"
                "sys.exit(mesh.main(sys.argv[1:]))\n" % str(ROOT / "mesh"))
            r = subprocess.run([sys.executable, str(stub), "teardown", "--yes", "--dry-run", "--json", "--config", str(Path(d) / "config.toml")],
                               capture_output=True, text=True, env={**os.environ, "CLOUDFLARE_API_TOKEN": "t"})
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(json.loads(r.stdout)["would_remove"], ["worker", "d1_id"])
            self.assertEqual(json.loads((Path(d) / "demo.state.json").read_text()), state)


if __name__ == "__main__":
    unittest.main()
