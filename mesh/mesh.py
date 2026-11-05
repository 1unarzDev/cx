#!/usr/bin/env python3
"""cx mesh: a WARP mesh on your own Cloudflare account, joined by one line (docs/mesh.md).

Owner side (a trusted machine: one holding the account's API token):
    mesh.py setup [--name N] [--domain D] [--account ID]   create or update everything (idempotent)
    mesh.py invite [--uses N] [--hours H] [--name DEVICE]   a short-lived code and the one-liners to send
    mesh.py devices | audit                                 what is on the mesh, what happened
    mesh.py revoke NAME [--yes]                             take a device (or a connector host) off the mesh
    mesh.py profile                                         the device profile setup applies, without applying
    mesh.py deploy                                          redeploy only the Worker and its schema (after a cx update)
    mesh.py teardown [--yes] [--dry-run]                    remove everything setup created
    mesh.py status                                          config and deployment, from local files only
    mesh.py pair-stash --lookup H --hours H                 hold a sealed `cx join` invite (stdin) for a short code
Onboarding (no config needed):
    mesh.py token-url [--with-domain]                       the dashboard link that creates the setup token
    mesh.py verify [--token-stdin]                          is the token valid? (GET /user/tokens/verify)
    mesh.py zones [--token-stdin]                           the zones (domains) the token can use
Device side (no cx needed; the one-liners do the same):
    mesh.py join CODE | mesh.py leave
    mesh.py pair-redeem --url URL --lookup H                take a short code's sealed invite (cx join does this)
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
import urllib.parse
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

# The setup token's dashboard template (docs/mesh.md, "Setup token"). Keys are the short permission-group
# keys of Cloudflare's API token template URLs (developers.cloudflare.com/fundamentals/api/how-to/
# account-owned-token-template/); user-token URLs accept only these keys and silently drop unknown ones.
TOKEN_TEMPLATE = "https://dash.cloudflare.com/profile/api-tokens"
TOKEN_PERMISSIONS = [  # (template key, type, the permission as docs/mesh.md names it)
    ("teams", "edit", "Zero Trust Write"),
    ("access", "edit", "Access: Apps and Policies Write"),
    ("access_acct", "read", "Access: Organizations, Identity Providers, and Groups Read"),
    ("d1", "edit", "D1 Write"),
    ("workers_scripts", "edit", "Workers Scripts Write"),
    ("account_api_tokens", "edit", "Account API Tokens Write"),
]
DOMAIN_PERMISSIONS = [("workers_routes", "edit", "Workers Routes Write"), ("zone", "read", "Zone Read")]
# No template key exists for these; the dashboard form needs them added by hand.
MANUAL_PERMISSIONS = ["Access: Service Tokens Write", "Cloudflare One Connector: WARP Write"]


def token_url(with_domain: bool = False, name: str = "cx-mesh") -> dict:
    """The dashboard link that opens a pre-filled "create token" form (no network, no secret)."""
    perms = TOKEN_PERMISSIONS + (DOMAIN_PERMISSIONS if with_domain else [])
    keys = json.dumps([{"key": k, "type": t} for k, t, _ in perms], separators=(",", ":"))
    q = urllib.parse.urlencode({"permissionGroupKeys": keys, "accountId": "*", "zoneId": "all", "name": name}, quote_via=urllib.parse.quote)
    return {"url": f"{TOKEN_TEMPLATE}?{q}", "permissions": [d for _, _, d in perms], "manual": MANUAL_PERMISSIONS, "name": name}


def request(method: str, url: str, token: str | None = None, body=None, timeout: int = 30) -> dict:
    """One JSON request; HTTP errors come back as {"success": False, "status": code, ...}."""
    data = json.dumps(body).encode() if body is not None else None
    h = {"user-agent": "cx-mesh", **({"authorization": f"Bearer {token}"} if token else {}), **({"content-type": "application/json"} if data is not None else {})}
    try:
        with urllib.request.urlopen(urllib.request.Request(url, method=method, data=data, headers=h), timeout=timeout) as r:
            return json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        text = e.read().decode(errors="replace")
        try:
            return {"success": False, "status": e.code, **json.loads(text)}
        except ValueError:
            return {"success": False, "status": e.code, "errors": [text[:300]]}


def verify_token(token: str, account: str | None = None) -> dict:
    """Is the token active? User tokens verify at /user/tokens/verify, account-owned ones under the account."""
    if not token:
        raise MeshError("no API token: set CLOUDFLARE_API_TOKEN or pass --token-stdin")
    urls = [("user", f"{API}/user/tokens/verify")] + ([("account", f"{API}/accounts/{account}/tokens/verify")] if account else [])
    last = {}
    for kind, url in urls:
        d = request("GET", url, token)
        r = d.get("result") or {}
        if d.get("success") and r.get("status"):
            return {"valid": r["status"] == "active", "status": r["status"], "id": r.get("id"), "expires_on": r.get("expires_on"), "kind": kind}
        last = d
    return {"valid": False, "status": "invalid", "id": None, "expires_on": None, "kind": None,
            "errors": [e.get("message", str(e)) if isinstance(e, dict) else str(e) for e in last.get("errors", [])][:3]}


def list_zones(token: str) -> dict:
    """The zones the token can see (one page of 50), for picking the mesh domain."""
    if not token:
        raise MeshError("no API token: set CLOUDFLARE_API_TOKEN or pass --token-stdin")
    d = request("GET", f"{API}/zones?per_page=50", token)
    if not d.get("success"):
        raise MeshError(f"GET /zones: {d.get('errors') or d.get('status')}")
    zones = [{"id": z["id"], "name": z["name"], "status": z.get("status"), "account": {"id": (z.get("account") or {}).get("id"), "name": (z.get("account") or {}).get("name")}} for z in d.get("result") or []]
    return {"zones": zones, "more": (d.get("result_info") or {}).get("total_pages", 1) > 1}


def status(cfg_path: str | None = None) -> dict:
    """Configured? Deployed? From the config and state files only."""
    path = config_path(cfg_path)
    if not path.exists():
        return {"configured": False, "provisioned": False, "config": str(path)}
    c = load_config(cfg_path)
    st_file = path.parent / f"{c['name']}.state.json"
    st = json.loads(st_file.read_text()) if st_file.exists() else {}
    return {"configured": True, "provisioned": bool(st.get("url")), "config": str(path), "name": c["name"], "domain": c.get("domain", ""),
            "endpoint": st.get("url"), "account": st.get("account") or c.get("account") or None, "mesh": c["profile"]["mesh"], "dns_suffix": c["dns_suffix"]}


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
        self.account = cfg.get("account") or self.state.get("account") or ""

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
        parts += [(f, f, "text/plain", (ASSETS / f).read_bytes()) for f in ("join.sh", "leave.sh", "join.ps1", "leave.ps1", "claude.sh", "claude.ps1")]
        b = uuid.uuid4().hex
        raw = b"".join(
            f'--{b}\r\nContent-Disposition: form-data; name="{n}"{f"; filename=\"{fn}\"" if fn else ""}\r\nContent-Type: {t}\r\n\r\n'.encode() + data + b"\r\n"
            for n, fn, t, data in parts) + f"--{b}--\r\n".encode()
        self.cf("PUT", f"/workers/scripts/{st['worker']}", raw=raw, ctype=f"multipart/form-data; boundary={b}")

    def deploy(self) -> dict:
        """The Worker and its D1 schema only: what a newer util changes, without touching the profile or policies."""
        st = self.state
        if not st.get("worker") or not st.get("d1_id"):
            raise MeshError("this mesh is not set up yet; run `cx mesh setup`")
        self.cf("POST", f"/d1/database/{st['d1_id']}/query", {"sql": (ASSETS / "schema.sql").read_text()})
        self.upload(None)
        self.say(f"worker {st['worker']} redeployed at {st['url']}")
        return {"worker": st["worker"], "url": st["url"]}

    # ---- the owner's everyday commands (through the Worker, authenticated by the account token)
    def invite(self, uses=1, hours=None, name=None) -> dict:
        return self.worker_api("POST", "/v1/admin/invites", {"uses": uses, "hours": hours or self.cfg.get("invite_ttl_hours", 24), **({"name": name} if name else {})})

    def devices(self) -> dict:
        return self.worker_api("GET", "/v1/admin/devices")

    def audit(self) -> dict:
        return self.worker_api("GET", "/v1/admin/audit")

    def revoke(self, name: str) -> dict:
        return self.worker_api("POST", f"/v1/admin/devices/{name}/revoke")

    def pair_stash(self, lookup: str, sealed: str, hours: float) -> dict:
        """Hold a `cx join` invite that cx sealed under its short code; the Worker sees only the seal."""
        return self.worker_api("POST", "/v1/admin/pairs", {"lookup": lookup, "sealed": sealed, "hours": hours})

    def teardown(self, dry_run: bool = False) -> dict:
        st, gone = self.state, []
        if dry_run:
            keys = ["domain_id", "worker", "d1_id", "worker_token_id", "policy_id", "sentinel_token_id", "profile_id"]
            return {"would_remove": [k for k in keys if st.get(k)], "devices": "every enrolled device is revoked first", "dry_run": True}
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


def pair_redeem(url: str, lookup: str) -> dict:
    """Take a short code's sealed invite, once (device side: no config, no token)."""
    if not url.startswith("https://"):
        raise MeshError("the mesh URL must be https")
    d = request("POST", url.rstrip("/") + "/v1/pair/redeem", None, {"lookup": lookup})
    if d.get("status", 200) >= 400 or d.get("error") or not d.get("sealed"):
        raise MeshError(d.get("error") or "the mesh endpoint gave no invite")
    return {"sealed": d["sealed"]}


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
    sp.add_parser("deploy")
    sp.add_parser("audit")
    sp.add_parser("profile")
    r = sp.add_parser("revoke")
    r.add_argument("name")
    r.add_argument("--yes", action="store_true")
    t = sp.add_parser("teardown")
    t.add_argument("--yes", action="store_true")
    t.add_argument("--dry-run", action="store_true", help="list what would be removed, remove nothing")
    sp.add_parser("status")
    tu = sp.add_parser("token-url")
    tu.add_argument("--with-domain", action="store_true", help="add the zone permissions a domain needs")
    tu.add_argument("--name", default="cx-mesh")
    for n in ("verify", "zones"):
        v = sp.add_parser(n)
        v.add_argument("--token-stdin", action="store_true", help="read the token from stdin instead of $CLOUDFLARE_API_TOKEN")
        v.add_argument("--account", default=None)
    ps = sp.add_parser("pair-stash")
    ps.add_argument("--lookup", required=True)
    ps.add_argument("--hours", type=float, default=1.0)
    ps.add_argument("--sealed-stdin", action="store_true")
    pr = sp.add_parser("pair-redeem")
    pr.add_argument("--url", required=True)
    pr.add_argument("--lookup", required=True)
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
        if a.cmd == "token-url":
            d = token_url(a.with_domain, a.name)
            out(d) if a.json else print(d["url"] + "\n\nAlso add by hand: " + ", ".join(d["manual"]))
            return 0
        if a.cmd in ("verify", "zones"):
            tok = sys.stdin.readline().strip() if a.token_stdin else os.environ.get("CLOUDFLARE_API_TOKEN", "").strip()
            if a.cmd == "verify":
                acct = a.account
                if acct is None and config_path(a.config).exists():
                    acct = load_config(a.config).get("account") or None
                d = verify_token(tok, acct)
                out(d) if a.json else print(f"{'valid' if d['valid'] else 'not valid'}: {d['status']}{' (' + d['kind'] + ' token)' if d['kind'] else ''}")
                return 0 if d["valid"] else 1
            d = list_zones(tok)
            if not a.json:
                for z in d["zones"]:
                    print(f"{z['name']:32} {z['status'] or '':10} {z['account']['name'] or ''}")
            out(d)
            return 0
        if a.cmd == "status":
            d = status(a.config)
            out(d) if a.json else print(json.dumps(d, indent=1))
            return 0
        if a.cmd == "pair-redeem":
            out(pair_redeem(a.url, a.lookup))
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
        elif a.cmd == "deploy":
            out(m.deploy())
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
        elif a.cmd == "pair-stash":
            sealed = sys.stdin.read().strip() if a.sealed_stdin else ""
            if not sealed:
                raise MeshError("pass the sealed invite on stdin (--sealed-stdin)")
            out(m.pair_stash(a.lookup, sealed, a.hours))
        elif a.cmd == "teardown" and a.dry_run:
            out(m.teardown(dry_run=True)) if a.json else print("would remove: " + ", ".join(m.teardown(dry_run=True)["would_remove"]))
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
