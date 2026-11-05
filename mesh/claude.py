#!/usr/bin/env python3
"""cx claude: lend your Claude Code gateway to a teammate with one line (docs/claude.md).

    claude.py lend NAME [--hours H | --days D] [--mesh-only]   a gateway key, an Access service token and its
                                                               policy, and a single-use code with the one-liners
    claude.py revoke NAME | --expired                          disable the key, delete the token and the policy
    claude.py list                                             the lends, their state and expiry
Every command takes --json and --config PATH (the mesh config: its [claude] table configures the gateway, and the
mesh's Worker serves the codes). The teammate needs no cx: `curl -sL <mesh endpoint>/claude/<code>|sh`.
Stdlib only, next to mesh.py.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import subprocess
import sys
import urllib.parse

from mesh import Mesh, MeshError, load_config, without_token

NAME_RE = re.compile(r"^[a-z0-9-]{1,40}$")
ANSI = re.compile(r"\x1b\[[0-9;]*m")
DEFAULTS = {"prefix": "cc-lend-", "days": 30, "code_hours": 24, "base_url": "", "mesh_url": "", "access_app": "", "gateway": []}
EXAMPLE = """[claude]
gateway = ["ssh", "gateway-host", "docker", "exec", "better-ccflare", "better-ccflare"]  # runs the better-ccflare CLI
base_url = "https://claude.example.org"   # the gateway behind Cloudflare Access (its Access app admits the lends)
mesh_url = ""                             # optional: the gateway on the mesh, e.g. "http://gateway.mesh.example.org:8081"
"""


def now() -> dt.datetime:
    return dt.datetime.now(dt.timezone.utc)


class Lends:
    def __init__(self, mesh: Mesh, run=subprocess.run):
        self.m, self.run = mesh, run
        c = {**DEFAULTS, **mesh.cfg.get("claude", {})}
        if not c["gateway"] or not (c["base_url"] or c["mesh_url"]):
            raise MeshError(f"add a [claude] table to {mesh.cfg['_path']}:\n{EXAMPLE}")
        self.c = c

    def say(self, msg: str) -> None:
        print(f"[claude] {msg}", file=self.m.out)

    # ---- the gateway (better-ccflare's own CLI)
    def gateway(self, *args: str) -> str:
        # The gateway command never needs the Cloudflare admin token, so it does not inherit it.
        r = self.run([*self.c["gateway"], *args], capture_output=True, text=True, timeout=90, env=without_token())
        out = ANSI.sub("", r.stdout + r.stderr)
        if r.returncode != 0 or "❌" in out or "Failed" in out:
            reason = next((l for l in out.splitlines() if "Failed" in l or "❌" in l or "rror" in l), f"exit {r.returncode}")
            raise MeshError(f"gateway {args[0]}: {re.sub(r'btr-[A-Za-z0-9_-]+', 'btr-…', reason.strip())}")
        return out

    def keys(self) -> dict[str, str]:
        """Gateway key name -> status (Active, Disabled ...)."""
        found, name = {}, None
        for line in self.gateway("--list-api-keys").splitlines():
            if m := re.match(r"^  (\S.*) \([A-Za-z0-9_-]+\)$", line):
                name = m.group(1)
            elif name and (m := re.match(r"^\s+Status:\s*(\S+)", line)):
                found[name] = m.group(1)
        return found

    def mint(self, key: str) -> str:
        if key in self.keys():  # a key's secret is shown only when generated: replace it to hand one out
            self.gateway("--delete-api-key", key)
        out = self.gateway("--generate-api-key", key)
        m = re.search(r"^Key:\s*(\S+)", out, re.M)
        if not m:
            raise MeshError("gateway --generate-api-key printed no key")
        return m.group(1)

    # ---- Cloudflare Access: the app in front of base_url, one service token and one app policy per lend
    def app(self) -> dict:
        want = self.c["access_app"] or urllib.parse.urlparse(self.c["base_url"]).hostname
        apps = self.m.cf("GET", "/access/apps?per_page=100") or []
        app = next((a for a in apps if a["id"] == want or (a.get("domain") or "").split("/")[0] == want), None)
        if not app:
            raise MeshError(f"no Access app for {want}; set claude.access_app to its id or domain")
        return app

    def access(self, full: str) -> tuple[dict, dict | None, dict | None]:
        app = self.app()
        pol = next((p for p in self.m.cf("GET", f"/access/apps/{app['id']}/policies") or [] if p["name"] == full), None)
        tok = next((t for t in self.m.cf("GET", f"/access/service_tokens?name={urllib.parse.quote(full)}&per_page=100") or [] if t["name"] == full), None)
        return app, pol, tok

    # ---- the commands
    def lend(self, name: str, hours: float | None = None, mesh_only: bool = False) -> dict:
        if not NAME_RE.match(name):
            raise MeshError("a lend name is lowercase letters, digits and -")
        if mesh_only and not self.c["mesh_url"]:
            raise MeshError("--mesh-only needs claude.mesh_url")
        if not mesh_only and not self.c["base_url"]:
            mesh_only = True
        hours = hours or self.c["days"] * 24
        full = self.c["prefix"] + name
        if self.revoke(name)["revoked"]:  # one lend per name: a new one replaces it
            self.say(f"{name} was lent already: replaced")
        expires = (now() + dt.timedelta(hours=hours)).replace(microsecond=0)
        try:
            key = self.mint(full)
            self.say(f"gateway key {full}")
            headers = []
            if not mesh_only:
                app, _, _ = self.access(full)
                tok = self.m.cf("POST", "/access/service_tokens", {"name": full, "duration": f"{max(1, round(hours))}h"})
                prec = max([p.get("precedence", 0) for p in self.m.cf("GET", f"/access/apps/{app['id']}/policies") or []] + [0]) + 1
                body = {"name": full, "decision": "non_identity", "include": [{"service_token": {"token_id": tok["id"]}}], "exclude": [], "require": [], "precedence": prec}
                self.m.cf("POST", f"/access/apps/{app['id']}/policies", body)
                headers = [f"CF-Access-Client-Id: {tok['client_id']}", f"CF-Access-Client-Secret: {tok['client_secret']}"]
                self.say(f"Access service token and policy {full} on {app.get('domain')}")
            bundle = {"base_url": "" if mesh_only else self.c["base_url"].rstrip("/"), "token": key, "headers": headers}
            d = self.m.worker_api("POST", "/v1/admin/claude", {
                "name": name, "bundle": bundle, "mesh_url": self.c["mesh_url"].rstrip("/"),
                "code_hours": self.c["code_hours"], "expires": expires.isoformat()})
        except BaseException:
            self.say("lend failed: undoing what it made")
            try:
                self.revoke(name)
            except Exception as e:  # noqa: BLE001
                self.say(f"undo incomplete ({e}); run `cx claude revoke {name}`")
            raise
        return {"name": name, "code": d["code"], "sh": d["sh"], "ps1": d["ps1"], "code_expires": d["code_expires"],
                "expires": expires.isoformat(), "route": "mesh" if mesh_only else ("mesh or access" if self.c["mesh_url"] else "access")}

    def revoke(self, name: str) -> dict:
        full, done = self.c["prefix"] + name, []
        if self.c["base_url"] or self.c["access_app"]:
            app, pol, tok = self.access(full)
            if pol:  # first: Access refuses to delete a token a policy names
                self.m.cf("DELETE", f"/access/apps/{app['id']}/policies/{pol['id']}", ok404=True)
                done.append("policy")
            if tok:
                self.m.cf("DELETE", f"/access/service_tokens/{tok['id']}", ok404=True)
                done.append("service token")
        if self.keys().get(full, "").lower() == "active":
            self.gateway("--disable-api-key", full)
            done.append("gateway key")
        if self.m.worker_api("POST", f"/v1/admin/claude/{name}/revoke").get("revoked"):
            done.append("code")
        return {"name": name, "revoked": done}

    def listing(self) -> dict[str, dict]:
        pre, keys = self.c["prefix"], self.keys()
        lends = {l["name"]: l for l in self.m.worker_api("GET", "/v1/admin/claude").get("lends", [])}
        for k, status in keys.items():
            if k.startswith(pre):
                lends.setdefault(k[len(pre):], {"name": k[len(pre):], "state": "untracked"})
        t = now().isoformat()
        for n, l in lends.items():
            l["key"] = keys.get(pre + n, "none")
            l["active"] = l["key"].lower() == "active" or l["state"] == "pending"
            l["expired"] = bool(l.get("expires")) and l["expires"] <= t
        return lends

    def expired(self) -> list[dict]:
        return [self.revoke(n) for n, l in self.listing().items() if l["expired"] and l["active"]]


def main(argv: list[str] | None = None) -> int:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--config", default=argparse.SUPPRESS)
    common.add_argument("--json", action="store_true", default=argparse.SUPPRESS)
    ap = argparse.ArgumentParser(prog="cx claude", description=__doc__.split("\n\n")[0], parents=[common])
    sp = ap.add_subparsers(dest="cmd", required=True)
    le = sp.add_parser("lend", parents=[common], help="a gateway key, Access token and policy, and a one-line code")
    le.add_argument("name")
    g = le.add_mutually_exclusive_group()
    g.add_argument("--hours", type=float)
    g.add_argument("--days", type=float)
    le.add_argument("--mesh-only", action="store_true", help="no Access token: the gateway only over the mesh")
    rv = sp.add_parser("revoke", parents=[common], help="disable the key, delete the token and the policy")
    rv.add_argument("name", nargs="?")
    rv.add_argument("--expired", action="store_true", help="every lend past its expiry")
    sp.add_parser("list", parents=[common], help="the lends")
    a = ap.parse_args(argv)
    a.config, a.json = getattr(a, "config", None), getattr(a, "json", False)
    try:
        L = Lends(Mesh(load_config(a.config)))
        if a.cmd == "lend":
            d = L.lend(a.name, a.hours or (a.days * 24 if a.days else None), a.mesh_only)
            if a.json:
                print(json.dumps(d, indent=1))
            else:
                print(f"Send this to {a.name} (single use, until {d['code_expires'][:16]}Z):\n"
                      f"  Linux/macOS:  {d['sh']}\n  Windows:      {d['ps1']}\n"
                      f"lend until {d['expires'][:16]}Z, route {d['route']}; end it with: cx claude revoke {a.name}")
        elif a.cmd == "revoke":
            if bool(a.name) == a.expired:
                raise MeshError("revoke NAME, or revoke --expired")
            d = {"revoked": L.expired()} if a.expired else L.revoke(a.name)
            if a.json:
                print(json.dumps(d, indent=1))
            elif a.expired:
                print("\n".join(f"{r['name']}: {', '.join(r['revoked']) or 'nothing'}" for r in d["revoked"]) or "no expired lends")
            else:
                print(f"{a.name}: {', '.join(d['revoked']) or 'nothing to revoke'}")
        else:
            d = L.listing()
            if a.json:
                print(json.dumps(list(d.values()), indent=1))
            for n, l in sorted(d.items()) if not a.json else []:
                exp = (l.get("expires") or "")[:16]
                print(f"{n:24} {l['state']:12} key {l['key']:9} {'until ' + exp + 'Z' if exp else ''}{' EXPIRED' if l['expired'] and l['active'] else ''}"
                      f"{'  claimed ' + l['claimed'][:16] + 'Z' if l.get('claimed') else ''}")
        return 0
    except (MeshError, subprocess.TimeoutExpired) as e:
        if a.json:
            print(json.dumps({"error": str(e)}))
        print(f"[claude] error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
