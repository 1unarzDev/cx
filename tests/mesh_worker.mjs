// The mesh share-code Worker (mesh/worker/worker.js) against an in-memory D1 (node:sqlite) and a fake
// Cloudflare API: invites, the one-liner scripts, enrolment, registration, the DNS name, leave and revoke, and Claude lends.
// No account is touched. Run: node tests/mesh_worker.mjs
import { DatabaseSync } from "node:sqlite";
import { readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { test } from "node:test";

const W = join(dirname(fileURLToPath(import.meta.url)), "..", "mesh", "worker");
const CONFIG = {
  name: "demo", account: "acct", org: "demo-team", url: "https://mesh.example.org", dns_suffix: ".mesh.example.org",
  rule_prefix: "Mesh DNS: ", policy_name: "demo-mesh enrolment", token_duration: "8760h", invite_ttl_hours: 24,
  devices: { robot: { ssh_user: "pilot" } }, ssh_keys: ["ssh-ed25519 AAAAexample owner"], tailscale_rule_priority: 5200,
};

// The Worker with its text-module imports inlined (Workers bundles them; node cannot import .sh).
const text = (f) => JSON.stringify(readFileSync(join(W, f), "utf8"));
const src = readFileSync(join(W, "worker.js"), "utf8")
  .replace(/^import (\w+) from "\.\/(join|leave|claude)\.(sh|ps1)";$/gm, (_, v, n, e) => `const ${v} = ${text(`${n}.${e}`)};`)
  .replace(/^import CONFIG_TEXT from "\.\/config\.json";.*$/m, `const CONFIG_TEXT = ${JSON.stringify(JSON.stringify(CONFIG))};`);
assert.ok(!/^import /m.test(src), "every Worker import is inlined");
const file = join(mkdtempSync(join(tmpdir(), "mesh-worker-")), "worker.mjs");
writeFileSync(file, src);
const worker = await import(file);

function d1() {
  const db = new DatabaseSync(":memory:");
  db.exec(readFileSync(join(W, "schema.sql"), "utf8"));
  const stmt = (sql, args = []) => ({
    bind: (...a) => stmt(sql, a),
    run: async () => ({ meta: { changes: Number(db.prepare(sql).run(...args).changes) } }),
    first: async () => (/\bRETURNING\b/i.test(sql) ? db.prepare(sql).get(...args) : db.prepare(sql).get(...args)) ?? null,
    all: async () => ({ results: db.prepare(sql).all(...args) }),
  });
  return { prepare: (sql) => stmt(sql), raw: db };
}

const OWNER = "owner-api-token";
/** The parts of the Cloudflare API the Worker calls, with just enough state to follow a device's life. */
function cloudflare() {
  const s = { tokens: new Map(), rules: new Map(), regs: [], connectors: [], policy: null, n: 0, calls: [] };
  const ok = (result) => new Response(JSON.stringify({ success: true, result }), { status: 200 });
  const no = (status) => new Response(JSON.stringify({ success: false, errors: [{ message: "no" }] }), { status });
  s.fetch = async (url, init = {}) => {
    const u = new URL(url);
    const m = init.method ?? "GET";
    const p = u.pathname.replace("/client/v4", "");
    const body = init.body ? JSON.parse(init.body) : undefined;
    const auth = (init.headers?.authorization ?? "").replace("Bearer ", "");
    s.calls.push(`${m} ${p}`);
    if (p.endsWith("/tokens/verify")) return auth === OWNER || auth === "worker-token" ? ok({ status: "active" }) : no(401);
    if (auth !== "worker-token") return no(403);
    const id = () => `id${++s.n}`;
    if (p === "/accounts/acct/access/service_tokens" && m === "POST") {
      const t = { id: id(), name: body.name, client_id: `cid-${s.n}`, client_secret: `secret-${s.n}` };
      s.tokens.set(t.id, t);
      return ok(t);
    }
    let r;
    if ((r = /^\/accounts\/acct\/access\/service_tokens\/(.+)$/.exec(p)) && m === "DELETE") {
      const named = s.policy?.include.some((i) => i.service_token.token_id === r[1]);
      if (named) return no(409); // Access refuses to delete a token a policy names
      return s.tokens.delete(r[1]) ? ok({}) : no(404);
    }
    if (/^\/accounts\/acct\/access\/policies\/pol$/.test(p) && m === "PUT") return ok((s.policy = body));
    if (p === "/accounts/acct/devices/registrations") return ok(s.regs);
    if (p === "/accounts/acct/devices/registrations/revoke") return ok({});
    if ((r = /^\/accounts\/acct\/devices\/registrations\/(.+)$/.exec(p)) && m === "DELETE") {
      s.regs = s.regs.filter((x) => x.id !== r[1]);
      return ok({});
    }
    if (p === "/accounts/acct/gateway/rules" && m === "GET") return ok([...s.rules.values()]);
    if (p === "/accounts/acct/gateway/rules" && m === "POST") {
      const x = { ...body, id: id() };
      s.rules.set(x.id, x);
      return ok(x);
    }
    if ((r = /^\/accounts\/acct\/gateway\/rules\/(.+)$/.exec(p))) {
      if (m === "DELETE") return s.rules.delete(r[1]) ? ok({}) : no(404);
      s.rules.set(r[1], { ...body, id: r[1] });
      return ok(s.rules.get(r[1]));
    }
    if (p === "/accounts/acct/warp_connector") return ok(s.connectors);
    if ((r = /^\/accounts\/acct\/warp_connector\/(.+)$/.exec(p)) && m === "DELETE") {
      s.connectors = s.connectors.filter((c) => c.id !== r[1]);
      return ok({});
    }
    return no(404);
  };
  return s;
}

function setup() {
  const cf = cloudflare();
  globalThis.fetch = cf.fetch;
  const env = { DB: d1(), CF_API_TOKEN: "worker-token", ENROL_POLICY_ID: "pol", SENTINEL_TOKEN_ID: "sentinel" };
  const call = async (method, path, { body, token, ua = "curl/8.10", ip = "198.51.100.7" } = {}) => {
    const headers = { "user-agent": ua, "cf-connecting-ip": ip };
    if (token) headers.authorization = `Bearer ${token}`;
    if (body) headers["content-type"] = "application/json";
    const r = await worker.default.fetch(new Request(`https://mesh.example.org${path}`, { method, headers, body: body && JSON.stringify(body) }), env);
    const t = await r.text();
    let j = null;
    try {
      j = JSON.parse(t);
    } catch {}
    return { status: r.status, text: t, json: j };
  };
  return { cf, env, call };
}

const invite = async (call, b = {}) => (await call("POST", "/v1/admin/invites", { token: OWNER, body: b })).json;

test("the owner endpoints need an active token of the account", async () => {
  const { call } = setup();
  assert.equal((await call("GET", "/v1/admin/devices")).status, 401);
  assert.equal((await call("GET", "/v1/admin/devices", { token: "stranger" })).status, 401);
  assert.equal((await call("GET", "/v1/admin/devices", { token: OWNER })).status, 200);
});

test("an invite's one-liners fetch the right bootstrap for each client", async () => {
  const { call } = setup();
  const inv = await invite(call);
  assert.match(inv.code, /^[2-9a-hjkmnp-z]{8}$/);
  assert.equal(inv.sh, `curl -sL mesh.example.org/${inv.code}|sh`);
  assert.equal(inv.ps1, `irm mesh.example.org/${inv.code}|iex`);
  const sh = await call("GET", `/${inv.code}`);
  assert.match(sh.text, /^#!\/bin\/sh/);
  assert.ok(sh.text.includes(`CODE='${inv.code}'`) && sh.text.includes("URL='https://mesh.example.org'"));
  assert.ok(!/__MESH_[A-Z_]+__/.test(sh.text), "every placeholder is filled");
  const ps = await call("GET", `/${inv.code}`, { ua: "Mozilla/5.0 (Windows NT; Windows NT 10.0) WindowsPowerShell/5.1" });
  assert.ok(ps.text.includes(`$Code = '${inv.code}'`));
  const page = await call("GET", `/${inv.code}`, { ua: "Mozilla/5.0 (X11; Linux x86_64) Firefox/140.0" });
  assert.match(page.text, /<!doctype html>/);
  assert.ok((await call("GET", "/leave")).text.includes("Leave the demo mesh"));
  assert.equal((await call("GET", "/v1/keys")).text, "ssh-ed25519 AAAAexample owner\n");
});

test("a device joins, gets its name, and leaves without a trace", async () => {
  const { cf, call } = setup();
  const inv = await invite(call);
  assert.equal((await call("POST", "/v1/enroll", { body: { code: "zzzzzzzz", name: "robot", host: "robot" } })).status, 403);
  const e = await call("POST", "/v1/enroll", { body: { code: inv.code, name: "Robot!", host: "robot", os: "Linux" } });
  assert.equal(e.status, 200, e.text);
  assert.equal(e.json.name, "robot");
  assert.equal(e.json.org, "demo-team");
  assert.equal(e.json.ssh_user, "pilot");
  assert.equal(e.json.fqdn, "robot.mesh.example.org");
  assert.deepEqual(cf.policy.include.map((i) => i.service_token.token_id).sort(), ["id1", "sentinel"]);
  assert.equal((await call("POST", "/v1/enroll", { body: { code: inv.code, name: "other", host: "other" } })).status, 403, "one use");

  const tok = e.json.device_token;
  assert.equal((await call("POST", "/v1/registered", { token: tok })).status, 404, "not registered yet");
  cf.regs.push({ id: "reg1", device: { name: "robot" }, user: { email: "non_identity@demo-team.cloudflareaccess.com" }, created_at: new Date().toISOString(), virtual_ipv4: "100.96.0.9", virtual_ipv6: "2606:4700:cf1:1000::9" });
  const reg = await call("POST", "/v1/registered", { token: tok });
  assert.equal(reg.status, 200, reg.text);
  const rule = [...cf.rules.values()][0];
  assert.equal(rule.name, "Mesh DNS: robot");
  assert.deepEqual(rule.rule_settings.override_ips, ["100.96.0.9", "2606:4700:cf1:1000::9"]);
  assert.equal(rule.traffic, 'dns.fqdn in {"robot" "robot.mesh.example.org"}');
  const dev = (await call("GET", "/v1/admin/devices", { token: OWNER })).json;
  assert.deepEqual(dev.enrolled.map((d) => [d.name, d.state, d.ipv4]), [["robot", "active", "100.96.0.9"]]);

  assert.equal((await call("POST", "/v1/leave", { token: tok })).status, 200);
  assert.equal(cf.rules.size, 0);
  assert.equal(cf.regs.length, 0);
  assert.deepEqual([...cf.tokens.keys()], [], "the device's service token is gone");
  assert.deepEqual(cf.policy.include.map((i) => i.service_token.token_id), ["sentinel"]);
  assert.equal((await call("POST", "/v1/leave", { token: tok })).status, 401, "the device token died with it");
  const audit = (await call("GET", "/v1/admin/audit", { token: OWNER })).json.audit.map((a) => a.action);
  assert.deepEqual(audit.reverse(), ["invite", "enroll", "registered", "leave"]);
});

test("a taken name is refused and the use refunded; a bound invite replaces the device", async () => {
  const { cf, call, env } = setup();
  cf.connectors.push({ id: "c1", name: "robot", status: "healthy" });
  const inv = await invite(call, { uses: 1 });
  const e = await call("POST", "/v1/enroll", { body: { code: inv.code, name: "robot", host: "robot" } });
  assert.equal(e.status, 409);
  assert.equal(env.DB.raw.prepare("SELECT uses_left FROM invites").get().uses_left, 1, "refunded");
  const bound = await invite(call, { name: "robot" });
  const b = await call("POST", "/v1/enroll", { body: { code: bound.code, name: "ignored", host: "robot-2" } });
  assert.equal(b.status, 200, b.text);
  assert.equal(b.json.name, "robot");
  assert.equal(cf.connectors.length, 0, "the old connector enrolment of that name is replaced");
});

test("the owner revokes a device by name", async () => {
  const { cf, call } = setup();
  const inv = await invite(call);
  const e = (await call("POST", "/v1/enroll", { body: { code: inv.code, name: "laptop", host: "laptop" } })).json;
  const r = await call("POST", "/v1/admin/devices/laptop/revoke", { token: OWNER });
  assert.deepEqual(r.json.revoked, ["device"]);
  assert.equal(cf.tokens.size, 0);
  assert.equal((await call("POST", "/v1/registered", { token: e.device_token })).status, 401);
});

test("joins are rate limited per address", async () => {
  const { call } = setup();
  for (let i = 0; i < 10; i++) await call("POST", "/v1/enroll", { body: { code: "zzzzzzzz" } });
  const r = await call("POST", "/v1/enroll", { body: { code: "zzzzzzzz" } });
  assert.equal(r.status, 429);
  assert.ok(Number(r.json.retry_after_s) > 0);
  assert.equal((await call("POST", "/v1/enroll", { body: { code: "zzzzzzzz" }, ip: "203.0.113.9" })).status, 403, "another address is not limited");
});

test("cleanName and scriptKind", () => {
  assert.equal(worker.cleanName("My Laptop.local"), "my-laptop-local");
  assert.equal(worker.scriptKind("curl/8.0"), "sh");
  assert.equal(worker.scriptKind("Wget/1.21"), "sh");
  assert.equal(worker.scriptKind("Mozilla/5.0 (Windows NT 10.0; Microsoft Windows 10.0.19045; en-US) PowerShell/7.4.0"), "ps1");
  assert.equal(worker.scriptKind("Mozilla/5.0 (Macintosh) Safari/605"), "html");
});

const BUNDLE = { base_url: "https://claude.example.org", token: "btr-secret-key", headers: ["CF-Access-Client-Id: cid.access", "CF-Access-Client-Secret: csecret"] };
const lend = async (call, b = {}) => call("POST", "/v1/admin/claude", { token: OWNER, body: { name: "evan", bundle: BUNDLE, mesh_url: "http://100.96.0.5:8081", expires: "2030-01-01T00:00:00Z", ...b } });

test("a Claude lend: sealed at rest, served as a script, claimed once", async () => {
  const { call, env } = setup();
  assert.equal((await call("POST", "/v1/admin/claude", { body: { name: "evan", bundle: BUNDLE } })).status, 401);
  assert.equal((await lend(call, { bundle: { ...BUNDLE, token: 'x"; rm -rf ~' } })).status, 400, "only plain values");
  const l = (await lend(call)).json;
  assert.match(l.code, /^[2-9a-hjkmnp-z]{12}$/);
  assert.equal(l.sh, `curl -sL mesh.example.org/claude/${l.code}|sh`);
  const row = env.DB.raw.prepare("SELECT * FROM lends").get();
  for (const v of [BUNDLE.token, "csecret", l.code]) assert.ok(!JSON.stringify(row).includes(v), "no secret or code at rest");
  const sh = await call("GET", `/claude/${l.code}`);
  assert.ok(sh.text.includes(`CODE='${l.code}'`) && sh.text.includes("MESH_URL='http://100.96.0.5:8081'") && sh.text.includes("MESH_ONLY=''"));
  assert.ok(!/__(MESH|CLAUDE)_[A-Z_]+__/.test(sh.text), "every placeholder is filled");
  assert.ok(!sh.text.includes(BUNDLE.token), "the script carries no secret");
  assert.ok((await call("GET", `/claude/${l.code}`, { ua: "Mozilla/5.0 PowerShell/7.4" })).text.includes(`$Code = '${l.code}'`));
  assert.match((await call("GET", `/claude/${l.code}`, { ua: "Mozilla/5.0 (X11) Firefox/140.0" })).text, /Set up Claude Code/);
  assert.equal((await call("POST", "/v1/claude/claim", { body: { code: "zzzzzzzzzzzz" } })).status, 403);
  const c = await call("POST", "/v1/claude/claim", { body: { code: l.code, format: "lines" } });
  assert.equal(c.status, 200, c.text);
  assert.equal(c.text, `name=evan\nbase_url=https://claude.example.org\nmesh_url=http://100.96.0.5:8081\ntoken=btr-secret-key\nheader=CF-Access-Client-Id: cid.access\nheader=CF-Access-Client-Secret: csecret\n`);
  assert.equal((await call("POST", "/v1/claude/claim", { body: { code: l.code } })).status, 403, "single use");
  assert.match((await call("GET", `/claude/${l.code}`)).text, /already used/);
  const list = (await call("GET", "/v1/admin/claude", { token: OWNER })).json.lends;
  assert.deepEqual(list.map((x) => [x.name, x.state, x.expires]), [["evan", "claimed", "2030-01-01T00:00:00.000Z"]]);
  assert.equal(env.DB.raw.prepare("SELECT sealed FROM lends").get().sealed, null);
});

test("a mesh-only lend says so to its script; revoke kills a pending code", async () => {
  const { call } = setup();
  const l = (await lend(call, { bundle: { ...BUNDLE, base_url: "", headers: [] } })).json;
  assert.ok((await call("GET", `/claude/${l.code}`)).text.includes("MESH_ONLY='1'"));
  assert.equal((await lend(call, { name: "x", bundle: { ...BUNDLE, base_url: "" }, mesh_url: "" })).status, 400, "a route is required");
  assert.deepEqual((await call("POST", "/v1/admin/claude/evan/revoke", { token: OWNER })).json, { name: "evan", revoked: true });
  assert.deepEqual((await call("POST", "/v1/admin/claude/evan/revoke", { token: OWNER })).json, { name: "evan", revoked: false }, "once");
  assert.equal((await call("POST", "/v1/claude/claim", { body: { code: l.code } })).status, 403);
  const c = (await lend(call)).json; // a new lend of the name replaces the old
  assert.equal((await call("POST", "/v1/claude/claim", { body: { code: c.code } })).json.token, BUNDLE.token);
});

test("an ssh-overlay short code: stashed sealed by the owner, redeemed once, never seen in clear", async () => {
  const { call, env } = setup();
  const lookup = "ab".repeat(32);
  const sealed = Buffer.from("sealed-by-cx ".repeat(4)).toString("base64");
  assert.equal((await call("POST", "/v1/admin/pairs", { body: { lookup, sealed } })).status, 401, "owner only");
  assert.equal((await call("POST", "/v1/admin/pairs", { token: OWNER, body: { lookup: "nothex", sealed } })).status, 400);
  assert.equal((await call("POST", "/v1/admin/pairs", { token: OWNER, body: { lookup, sealed: "not base64!" } })).status, 400);
  const r = await call("POST", "/v1/admin/pairs", { token: OWNER, body: { lookup, sealed, hours: 2 } });
  assert.equal(r.status, 200);
  assert.equal(r.json.url, "https://mesh.example.org");
  assert.equal((await call("POST", "/v1/admin/pairs", { token: OWNER, body: { lookup, sealed } })).status, 409, "a lookup is stashed once");
  assert.equal((await call("POST", "/v1/pair/redeem", { body: { lookup: "cd".repeat(32) } })).status, 403);
  const got = await call("POST", "/v1/pair/redeem", { body: { lookup }, ip: "203.0.113.9" });
  assert.equal(got.status, 200);
  assert.equal(got.json.sealed, sealed);
  assert.equal((await call("POST", "/v1/pair/redeem", { body: { lookup }, ip: "203.0.113.9" })).status, 403, "single use");
  const row = env.DB.raw.prepare("SELECT * FROM pairs").get();
  assert.equal(row.sealed, null);
  assert.equal(row.state, "redeemed");
  assert.equal(row.redeemed_ip, "203.0.113.9");
  const page = await call("GET", "/p/ABCD-EFGH-JKMN");
  assert.ok(page.text.includes("cx join mesh.example.org/ABCD-EFGH-JKMN"));
  const audit = (await call("GET", "/v1/admin/audit", { token: OWNER })).json.audit.map((a) => a.action);
  assert.ok(audit.includes("pair") && audit.includes("redeem"));
});

test("an expired short code cannot be redeemed, and redeems are rate limited", async () => {
  const { call, env } = setup();
  const lookup = "ef".repeat(32);
  await call("POST", "/v1/admin/pairs", { token: OWNER, body: { lookup, sealed: Buffer.from("x".repeat(40)).toString("base64") } });
  env.DB.raw.prepare("UPDATE pairs SET expires = '2000-01-01T00:00:00.000Z'").run();
  assert.equal((await call("POST", "/v1/pair/redeem", { body: { lookup } })).status, 403);
  let last;
  for (let i = 0; i < 12; i++) last = await call("POST", "/v1/pair/redeem", { body: { lookup: "00".repeat(32) }, ip: "192.0.2.50" });
  assert.equal(last.status, 429);
});
