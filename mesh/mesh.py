#!/usr/bin/env python3
"""cx mesh: a WARP mesh on your own Cloudflare account, joined by one line (docs/mesh.md).

Owner side (a trusted machine: one holding the account's API token):
    mesh.py setup [--name N] [--domain D] [--account ID]   create or update everything (idempotent)
    mesh.py invite [--uses N] [--hours H] [--name DEVICE]   a short-lived code and the one-liners to send
    mesh.py devices | audit                                 what is on the mesh, what happened
    mesh.py revoke NAME [--yes]                             take a device (or a connector host) off the mesh
    mesh.py profile                                         the device profile setup applies, without applying
    mesh.py teardown [--yes]                                remove everything setup created
Device side (no cx needed; the one-liners do the same):
    mesh.py join CODE | mesh.py leave
Every command takes --json (machine-readable output) and --config PATH (default
$XDG_CONFIG_HOME/cx/mesh/config.toml). The API token comes from $CLOUDFLARE_API_TOKEN, else the credentials file
setup writes (mode 600). Stdlib only; usable as a library: `Mesh(load_config()).devices()`.
"""
from __future__ import annotations

import argparse
import getpass
import ipaddress
import json
import os
import subprocess
import sys
import tomllib
import urllib.error
import urllib.request
import uuid
from pathlib import Path

API = "https://api.cloudflare.com/client/v4"
HERE = Path(__file__).resolve().parent
ASSETS = HERE / "worker"
CONFIG_DIR = Path(os.environ.get("XDG_CONFIG_HOME", "~/.config")).expanduser() / "cx" / "mesh"
COMPAT_DATE = "2026-10-01"
# What the Worker's own token may do (setup creates it scoped to the account).
WORKER_SCOPES = ["Zero Trust Write", "Access: Service Tokens Write", "Access: Apps and Policies Write", "Cloudflare One Connector: WARP Write"]

DEFAULT_PROFILE = {
    "precedence": 20,
    "tunnel_protocol": "masque",
    "mesh": ["100.96.0.0/12"],
    "tunneled_lans": [],
    "exclude": [
        {"address": a, "description": d}
        for a, d in [
            ("10.0.0.0/8", "LAN"), ("172.16.0.0/12", "LAN"), ("192.168.0.0/16", "LAN"),
            ("169.254.0.0/16", "link-local"), ("224.0.0.0/4", "multicast (mDNS)"), ("240.0.0.0/4", "reserved"),
            ("127.0.0.0/8", "loopback"), ("100.64.0.0/10", "CGNAT outside the mesh: Tailscale and other VPNs"),
            ("fe80::/10", "link-local"), ("fc00::/7", "ULA, Tailscale's included"), ("ff00::/8", "multicast"), ("::1/128", "loopback"),
            ("198.41.192.0/24", "cloudflared transport"), ("198.41.200.0/24", "cloudflared transport"),
            ("2606:4700:a0::/48", "cloudflared transport"), ("2606:4700:a8::/48", "cloudflared transport"),
        ]
    ],
}

TEMPLATE = """# cx mesh instance (docs/mesh.md). Edit, then run `cx mesh setup` again to apply.
name = "{name}"                 # names the Cloudflare resources: worker/D1 {name}-mesh, profile {name}-mesh
account = "{account}"
domain = "{domain}"             # with a domain the endpoint is <endpoint>.<domain>; empty: workers.dev
endpoint = "mesh"
dns_suffix = "{dns_suffix}"     # devices resolve as <device><dns_suffix> for mesh members (Gateway DNS overrides)
rule_prefix = "Mesh DNS: "
match = ""                      # which devices get the profile; empty: every service-token (joined) device
invite_ttl_hours = 24
token_duration = "8760h"        # a device's enrolment service token (revoked on leave or revoke)
ssh_keys = []                   # public keys a join adds to the joining user's authorized_keys (leave removes them)

[devices]                       # optional, per device: ssh_user = "..." (the ssh line the join prints)

[profile]
precedence = 20
tunnel_protocol = "masque"
mesh = ["100.96.0.0/12"]        # carved out of the excludes: rides WARP
tunneled_lans = []              # private networks that ride the mesh anyway (a device on one needs a LAN-priority helper)
exclude = [
{excludes}
]
"""


class MeshError(Exception):
    pass


def config_path(p: str | None = None) -> Path:
    return Path(p).expanduser() if p else CONFIG_DIR / "config.toml"


def load_config(p: str | None = None) -> dict:
    path = config_path(p)
    if not path.exists():
        raise MeshError(f"no mesh config at {path}; run `cx mesh setup` first")
    cfg = tomllib.loads(path.read_text())
    cfg["_path"] = str(path)
    cfg["profile"] = {**DEFAULT_PROFILE, **cfg.get("profile", {})}
    cfg.setdefault("endpoint", "mesh")
    cfg.setdefault("rule_prefix", "Mesh DNS: ")
    cfg.setdefault("dns_suffix", f".{cfg['endpoint']}.{cfg['domain']}" if cfg.get("domain") else ".mesh")
    return cfg


def write_config(path: Path, name: str, account: str, domain: str) -> None:
    ex = ",\n".join(f'  {{address = "{e["address"]}", description = "{e["description"]}"}}' for e in DEFAULT_PROFILE["exclude"])
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(TEMPLATE.format(name=name, account=account, domain=domain, dns_suffix=f".mesh.{domain}" if domain else ".mesh", excludes=ex))


def excludes(profile: dict) -> list[dict]:
    """The profile's exclude list with the mesh and any tunneled LANs carved out (they ride WARP)."""
    carve = [ipaddress.ip_network(n) for n in [*profile["mesh"], *profile.get("tunneled_lans", [])]]
    out: list[dict] = []
    for e in profile["exclude"]:
        nets = [ipaddress.ip_network(e["address"])]
        for c in carve:
            nxt = []
            for n in nets:
                if n.version == c.version and c.subnet_of(n):
                    nxt += list(n.address_exclude(c))
                elif n.version == c.version and n.subnet_of(c):
                    continue
                else:
                    nxt.append(n)
            nets = nxt
        out += [{"address": str(n), "description": e.get("description", "")} for n in sorted(nets)]
    return out


class Mesh:
    def __init__(self, cfg: dict, token: str | None = None, out=sys.stderr, dry_run: bool = False):
        self.cfg, self.out, self.dry_run = cfg, out, dry_run
        self.planned: list[str] = []
        self.dir = Path(cfg["_path"]).parent
        self.state_file = self.dir / f"{cfg['name']}.state.json"
        self.state = json.loads(self.state_file.read_text()) if self.state_file.exists() else {}
        self._token = token
        self.account = cfg.get("account") or ""

    # ---- plumbing
    def say(self, m: str) -> None:
        print(f"[mesh] {m}", file=self.out)

    def save(self) -> None:
        if self.dry_run:
            return
        self.state_file.write_text(json.dumps(self.state, indent=1))
        self.state_file.chmod(0o600)

    @property
    def creds(self) -> Path:
        return self.dir / "credentials"

    def token(self) -> str:
        if self._token:
            return self._token
        t = os.environ.get("CLOUDFLARE_API_TOKEN") or (self.creds.read_text().strip() if self.creds.exists() else "")
        if not t:
            if not sys.stdin.isatty():
                raise MeshError("no API token: set CLOUDFLARE_API_TOKEN or run `cx mesh setup` in a terminal")
            t = getpass.getpass("Cloudflare API token (docs/mesh.md lists its scopes; not echoed): ").strip()
            self.creds.parent.mkdir(parents=True, exist_ok=True)
            self.creds.touch(0o600)
            self.creds.write_text(t + "\n")
        self._token = t
        return t

    def http(self, method: str, url: str, body=None, headers=None, raw: bytes | None = None, ctype="application/json"):
        data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
        h = {"authorization": f"Bearer {self.token()}", "user-agent": "cx-mesh", **({"content-type": ctype} if data is not None else {}), **(headers or {})}
        req = urllib.request.Request(url, method=method, data=data, headers=h)
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                return json.loads(r.read() or b"{}")
        except urllib.error.HTTPError as e:
            text = e.read().decode(errors="replace")
            try:
                return {"success": False, "status": e.code, **json.loads(text)}
            except ValueError:
                return {"success": False, "status": e.code, "errors": [text[:300]]}

    def cf(self, method: str, path: str, body=None, ok404=False, **kw):
        base = f"{API}/accounts/{self.account}" if not path.startswith("/zones") and not path.startswith("/user") else API
        if self.dry_run and method != "GET":
            self.planned.append(f"{method} {path.split('?')[0]}")
            self.say(f"would {method} {path.split('?')[0]}")
            return {"id": "dry-run", "policy_id": "dry-run", "uuid": "dry-run", "value": "dry-run", "policies": []}
        d = self.http(method, base + path, body, **kw)
        if d.get("status") == 404 and ok404:
            return None
        if not d.get("success", True) or d.get("status", 200) >= 400:
            raise MeshError(f"{method} {path.split('?')[0]}: {d.get('errors') or d}")
        return d.get("result")

    def worker_api(self, method: str, path: str, body=None):
        d = self.http(method, self.url() + path, body)
        if d.get("status", 200) >= 400 or d.get("error"):
            raise MeshError(" ".join(filter(None, [d.get("error"), d.get("detail")])) or str(d))
        return d

    def url(self) -> str:
        if not self.state.get("url"):
            raise MeshError("this mesh has no endpoint yet; run `cx mesh setup`")
        return self.state["url"]

    def team(self) -> str:
        org = self.cf("GET", "/access/organizations")
        if not org or not org.get("auth_domain"):
            raise MeshError("this account has no Zero Trust organization: open one.dash.cloudflare.com once and pick the Free plan")
        return org["auth_domain"].split(".")[0]

    # ---- setup
    def profile_body(self, team: str) -> dict:
        p = self.cfg["profile"]
        return {
            "name": f"{self.cfg['name']}-mesh",
            "description": f"cx mesh {self.cfg['name']}: internet over WARP, LANs excluded, the mesh tunneled",
            "match": self.cfg.get("match") or f'identity.email == "non_identity@{team}.cloudflareaccess.com"',
            "precedence": p["precedence"],
            "enabled": True,
            "tunnel_protocol": p["tunnel_protocol"],
            "service_mode_v2": {"mode": "warp"},
            "allowed_to_leave": True,
            "allow_mode_switch": False,
            "switch_locked": False,
            "auto_connect": 0,
            "captive_portal": 180,
        }

    def setup(self) -> dict:
        c, st = self.cfg, self.state
        if not self.account:
            accts = self.http("GET", f"{API}/accounts?per_page=5").get("result") or []
            if len(accts) != 1:
                raise MeshError("set account = \"<id>\" in the config (the token reaches %d accounts)" % len(accts))
            self.account = accts[0]["id"]
        team = self.team()
        st.update(account=self.account, team=team, name=c["name"])
        # 1. mesh routing between devices (virtual IPs, the Gateway proxy)
        s = self.cf("GET", "/devices/settings") or {}
        want = {"use_zt_virtual_ip": True, "gateway_proxy_enabled": True, "gateway_udp_proxy_enabled": True}
        if any(s.get(k) != v for k, v in want.items()):
            self.cf("PUT", "/devices/settings", {**s, **want})
            self.say("device settings: virtual IPs and the Gateway proxy on")
        # 2. the device profile
        body = self.profile_body(team)
        found = next((p for p in self.cf("GET", "/devices/policies") or [] if p.get("name") == body["name"]), None)
        pid = found["policy_id"] if found else self.cf("POST", "/devices/policy", body)["policy_id"]
        if found:
            self.cf("PATCH", f"/devices/policy/{pid}", body)
        self.cf("PUT", f"/devices/policy/{pid}/exclude", excludes(c["profile"]))
        st["profile_id"] = pid
        self.save()
        self.say(f"profile {body['name']}: {c['profile']['tunnel_protocol']}, {len(excludes(c['profile']))} excludes")
        # 3. enrolment: a reusable Service Auth policy on the WARP login app; the Worker keeps its include list
        sname = f"{c['name']}-mesh-sentinel"
        tokens = self.cf("GET", "/access/service_tokens?per_page=100") or []
        sent = next((t for t in tokens if t["name"] == sname), None) or self.cf("POST", "/access/service_tokens", {"name": sname, "duration": "forever"})
        st["sentinel_token_id"] = sent["id"]
        self.save()
        pname = f"{c['name']}-mesh enrolment"
        pol = next((p for p in self.cf("GET", "/access/policies?per_page=100") or [] if p["name"] == pname), None)
        if not pol:
            pol = self.cf("POST", "/access/policies", {"name": pname, "decision": "non_identity", "include": [{"service_token": {"token_id": sent["id"]}}], "exclude": [], "require": []})
        st["policy_id"] = pol["id"]
        self.save()
        app = next((a for a in self.cf("GET", "/access/apps?per_page=100") or [] if a.get("type") == "warp"), None)
        if not app:
            app = self.cf("POST", "/access/apps", {"type": "warp", "name": "Warp Login App", "session_duration": "24h", "policies": [{"id": pol["id"], "precedence": 1}]})
            st["created_app"] = True
        elif pol["id"] not in [p["id"] for p in app.get("policies", [])]:
            self.attach(app, pol["id"], add=True)
        st["warp_app_id"] = app["id"]
        self.say(f"enrolment policy {pname} on the WARP login app")
        # 4. state: D1
        dname = f"{c['name']}-mesh"
        db = next((d for d in self.cf("GET", f"/d1/database?name={dname}") or [] if d["name"] == dname), None) or self.cf("POST", "/d1/database", {"name": dname})
        st["d1_id"] = db.get("uuid") or db.get("id")
        self.save()
        self.cf("POST", f"/d1/database/{st['d1_id']}/query", {"sql": (ASSETS / "schema.sql").read_text()})
        # 5. the Worker: its URL, its own scoped token, the upload
        st["worker"] = dname
        if c.get("domain"):
            host = f"{c['endpoint']}.{c['domain']}"
            st["url"] = f"https://{host}"
        else:
            sub = (self.cf("GET", "/workers/subdomain") or {}).get("subdomain")
            if not sub:
                raise MeshError("this account has no workers.dev subdomain: open Workers once in the dashboard, or set a domain")
            st["url"] = f"https://{dname}.{sub}.workers.dev"
        secret = None
        if not st.get("worker_token_id"):
            secret = self.worker_token()
        self.upload(secret)
        if c.get("domain"):
            zone = (self.cf("GET", f"/zones?name={c['domain']}") or [None])[0]
            if not zone:
                raise MeshError(f"the domain {c['domain']} is not a zone on this account")
            d = self.cf("PUT", "/workers/domains", {"environment": "production", "hostname": host, "service": dname, "zone_id": zone["id"]})
            st["domain_id"] = d["id"]
        else:
            self.cf("POST", f"/workers/scripts/{dname}/subdomain", {"enabled": True})
        self.save()
        self.say(f"endpoint {st['url']}; next: cx mesh invite")
        return {**st, "planned": self.planned} if self.dry_run else dict(st)

    def attach(self, app: dict, pid: str, add: bool) -> None:
        keep = [p["id"] for p in app.get("policies", []) if p["id"] != pid] + ([pid] if add else [])
        body = {k: v for k, v in app.items() if k not in ("id", "uid", "aud", "created_at", "updated_at", "policies")}
        body["policies"] = [{"id": i, "precedence": n + 1} for n, i in enumerate(keep)]
        self.cf("PUT", f"/access/apps/{app['id']}", body)

    def worker_token(self) -> str:
        groups = self.cf("GET", "/tokens/permission_groups") or []
        found = {g["name"]: g["id"] for g in groups if g["name"] in WORKER_SCOPES and "com.cloudflare.api.account" in g.get("scopes", [])}
        if set(found) != set(WORKER_SCOPES):
            raise MeshError(f"the setup token cannot see the permission groups {sorted(set(WORKER_SCOPES) - set(found))}")
        ids = [{"id": i} for i in found.values()]
        body = {"name": f"{self.cfg['name']}-mesh worker", "policies": [{"effect": "allow", "permission_groups": ids, "resources": {f"com.cloudflare.api.account.{self.account}": "*"}}]}
        try:
            t = self.cf("POST", "/tokens", body)
            self.state["worker_token"] = "account"
        except MeshError:
            t = self.cf("POST", "/user/tokens", body)  # a user token, when the setup token cannot mint account tokens
            self.state["worker_token"] = "user"
        self.state["worker_token_id"] = t["id"]
        self.save()  # before anything can fail: teardown must find it
        return t["value"]

    def worker_config(self) -> dict:
        c = self.cfg
        return {
            "name": c["name"], "account": self.account, "org": self.state["team"], "url": self.state["url"],
            "dns_suffix": c["dns_suffix"], "rule_prefix": c["rule_prefix"], "policy_name": f"{c['name']}-mesh enrolment",
            "token_duration": c.get("token_duration", "8760h"), "invite_ttl_hours": c.get("invite_ttl_hours", 24),
            "devices": c.get("devices", {}), "ssh_keys": c.get("ssh_keys", []), "tailscale_rule_priority": c.get("tailscale_rule_priority", 5200),
        }

    def upload(self, secret: str | None) -> None:
        st = self.state
        bindings = [
            {"type": "d1", "name": "DB", "id": st["d1_id"]},
            {"type": "plain_text", "name": "ENROL_POLICY_ID", "text": st["policy_id"]},
            {"type": "plain_text", "name": "SENTINEL_TOKEN_ID", "text": st["sentinel_token_id"]},
        ]
        if secret:
            bindings.append({"type": "secret_text", "name": "CF_API_TOKEN", "text": secret})
        meta = {"main_module": "worker.js", "compatibility_date": COMPAT_DATE, "bindings": bindings, "keep_bindings": ["secret_text"]}
        parts = [("metadata", "", "application/json", json.dumps(meta).encode()),
                 ("worker.js", "worker.js", "application/javascript+module", (ASSETS / "worker.js").read_bytes()),
                 ("config.json", "config.json", "text/plain", json.dumps(self.worker_config()).encode())]
        parts += [(f, f, "text/plain", (ASSETS / f).read_bytes()) for f in ("join.sh", "leave.sh", "join.ps1", "leave.ps1")]
        b = uuid.uuid4().hex
        raw = b"".join(
            f'--{b}\r\nContent-Disposition: form-data; name="{n}"{f"; filename=\"{fn}\"" if fn else ""}\r\nContent-Type: {t}\r\n\r\n'.encode() + data + b"\r\n"
            for n, fn, t, data in parts) + f"--{b}--\r\n".encode()
        self.cf("PUT", f"/workers/scripts/{st['worker']}", raw=raw, ctype=f"multipart/form-data; boundary={b}")

    # ---- the owner's everyday commands (through the Worker, authenticated by the account token)
    def invite(self, uses=1, hours=None, name=None) -> dict:
        return self.worker_api("POST", "/v1/admin/invites", {"uses": uses, "hours": hours or self.cfg.get("invite_ttl_hours", 24), **({"name": name} if name else {})})

    def devices(self) -> dict:
        return self.worker_api("GET", "/v1/admin/devices")

    def audit(self) -> dict:
        return self.worker_api("GET", "/v1/admin/audit")

    def revoke(self, name: str) -> dict:
        return self.worker_api("POST", f"/v1/admin/devices/{name}/revoke")

    def teardown(self) -> dict:
        st, gone = self.state, []
        if st.get("url"):
            try:
                for d in self.devices().get("enrolled", []):
                    if d["state"] in ("enrolling", "active"):
                        self.revoke(d["name"])
                        gone.append(f"device {d['name']}")
            except MeshError as e:
                self.say(f"devices not revoked through the endpoint ({e}); continuing")
        steps = [
            ("domain_id", lambda v: self.cf("DELETE", f"/workers/domains/{v}", ok404=True)),
            ("worker", lambda v: self.cf("DELETE", f"/workers/scripts/{v}?force=true", ok404=True)),
            ("d1_id", lambda v: self.cf("DELETE", f"/d1/database/{v}", ok404=True)),
            ("worker_token_id", lambda v: self.cf("DELETE", f"/{'user/' if st.get('worker_token') == 'user' else ''}tokens/{v}".replace("//", "/"), ok404=True)),
            ("policy_id", self._drop_policy),
            ("sentinel_token_id", lambda v: self.cf("DELETE", f"/access/service_tokens/{v}", ok404=True)),
            ("profile_id", lambda v: self.cf("DELETE", f"/devices/policy/{v}", ok404=True)),
        ]
        for key, fn in steps:
            if st.get(key):
                fn(st[key])
                gone.append(key)
        self.state_file.unlink(missing_ok=True)
        return {"removed": gone}

    def _drop_policy(self, pid: str) -> None:
        app = self.cf("GET", f"/access/apps/{self.state['warp_app_id']}", ok404=True)
        if app and self.state.get("created_app"):
            self.cf("DELETE", f"/access/apps/{app['id']}", ok404=True)
        elif app:
            self.attach(app, pid, add=False)
        self.cf("DELETE", f"/access/policies/{pid}", ok404=True)


def device_script(url: str, leaving: bool) -> int:
    """The device side: what the one-liner runs (curl ... | sh)."""
    sh = subprocess.run(["curl", "-fsSL", url], capture_output=True, check=True).stdout
    return subprocess.run(["sh"], input=sh).returncode


def main(argv: list[str] | None = None) -> int:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--config", default=argparse.SUPPRESS)
    common.add_argument("--json", action="store_true", default=argparse.SUPPRESS)
    ap = argparse.ArgumentParser(prog="cx mesh", description=__doc__.split("\n\n")[0], parents=[common])
    sp = ap.add_subparsers(dest="cmd", required=True)
    add = sp.add_parser

    def sub(name):
        return add(name, parents=[common])

    sp.add_parser = sub
    s = sp.add_parser("setup")
    s.add_argument("--dry-run", action="store_true", help="read the account, print the changes, apply none")
    s.add_argument("--name")
    s.add_argument("--domain", default=None)
    s.add_argument("--account", default="")
    i = sp.add_parser("invite")
    i.add_argument("--uses", type=int, default=1)
    i.add_argument("--hours", type=float)
    i.add_argument("--name", help="bind the code to this device name (a re-join replaces that device)")
    sp.add_parser("devices")
    sp.add_parser("audit")
    sp.add_parser("profile")
    r = sp.add_parser("revoke")
    r.add_argument("name")
    r.add_argument("--yes", action="store_true")
    t = sp.add_parser("teardown")
    t.add_argument("--yes", action="store_true")
    j = sp.add_parser("join")
    j.add_argument("code", help="the invite code, or the whole URL")
    sp.add_parser("leave").add_argument("--url")
    a = ap.parse_args(argv)
    a.config, a.json = getattr(a, "config", None), getattr(a, "json", False)
    out = lambda d: print(json.dumps(d, indent=1)) if a.json else None  # noqa: E731
    try:
        if a.cmd == "setup":
            path = config_path(a.config)
            if not path.exists():
                if not a.name:
                    raise MeshError("first setup: pass --name (and --domain if you have one on this account)")
                write_config(path, a.name, a.account, a.domain or "")
                print(f"[mesh] wrote {path}", file=sys.stderr)
            if a.dry_run and not path.exists():
                raise MeshError("a dry run needs a config: run `cx mesh setup --name N` once, or pass --config")
            out(Mesh(load_config(a.config), dry_run=a.dry_run).setup())
            return 0
        if a.cmd in ("join", "leave"):
            if a.cmd == "join":
                url = a.code if "/" in a.code else f"{Mesh(load_config(a.config)).url()}/{a.code}"
            else:
                url = (a.url or Mesh(load_config(a.config)).url()) + "/leave"
            return device_script(url if url.startswith("http") else f"https://{url}", a.cmd == "leave")
        m = Mesh(load_config(a.config))
        if a.cmd == "profile":
            d = {"excludes": excludes(m.cfg["profile"]), "profile": m.profile_body(m.state.get("team", "<team>"))}
            out(d) if a.json else print(json.dumps(d, indent=1))
        elif a.cmd == "invite":
            d = m.invite(a.uses, a.hours, a.name)
            if not out(d) and not a.json:
                print(f"Linux/macOS:  {d['sh']}\nWindows:      {d['ps1']}\nexpires {d['expires']}, {d['uses']} use(s)"
                      f"{', bound to ' + d['name'] if d.get('name') else ''}\nleave:        {d['leave_sh']}  |  {d['leave_ps1']}")
        elif a.cmd == "devices":
            d = m.devices()
            if not a.json:
                rows = [(e["name"], "joined", e["state"], " ".join(filter(None, [e.get("ipv4"), e.get("ipv6")]))) for e in d["enrolled"]]
                rows += [(c["name"], "connector", c["status"], "") for c in d["connectors"]]
                rows += [(r["name"], "dns only", "", "") for r in d["dns"] if r["name"] not in {x[0] for x in rows}]
                ips = {r["name"]: " ".join(r["ips"]) for r in d["dns"]}
                for n, kind, state, addr in sorted(rows):
                    print(f"{n:24} {kind:10} {state:10} {addr or ips.get(n, '')}  {n + d['dns_suffix'] if n in ips else ''}")
            out(d)
        elif a.cmd == "audit":
            d = m.audit()
            if not a.json:
                for e in d["audit"]:
                    print(f"{e['at'][:19]} {e['actor']:22} {e['action']:12} {e['detail'] or ''}")
            out(d)
        elif a.cmd == "revoke":
            if not a.yes:
                if not sys.stdin.isatty() or input(f"take {a.name} off the mesh (its registration, token, DNS name)? type the name: ") != a.name:
                    raise MeshError("not confirmed (pass --yes)")
            d = m.revoke(a.name)
            out(d) if a.json else print(f"{a.name}: {', '.join(d['revoked']) or 'nothing to revoke'}")
        elif a.cmd == "teardown":
            if not a.yes and (not sys.stdin.isatty() or input(f"remove the {m.cfg['name']} mesh and every device it enrolled? type the name: ") != m.cfg["name"]):
                raise MeshError("not confirmed (pass --yes)")
            d = m.teardown()
            out(d) if a.json else print("removed: " + ", ".join(d["removed"]))
        return 0
    except MeshError as e:
        if a.json:
            print(json.dumps({"error": str(e)}))
        print(f"[mesh] error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
