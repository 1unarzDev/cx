// The share-code endpoint of a WARP mesh (README.md): `curl -sL <url>/<code>|sh`, `irm <url>/<code>|iex` and
// `curl -sL <url>/leave|sh`. `mesh setup` deploys it with the instance's config (config.json) and secrets:
//   CF_API_TOKEN       a token scoped to the account's Zero Trust (service tokens, the enrolment policy,
//                      registrations, Gateway rules, WARP connectors)
//   vars               ENROL_POLICY_ID, SENTINEL_TOKEN_ID (the policy's never-handed-out placeholder token)
// - GET / and /<code> answer with the bootstrap the client wants (PowerShell: join.ps1; a browser: a page;
//   anything else: join.sh); /leave the same for leave.*; /v1/keys the ssh keys a join authorizes.
// - POST /v1/enroll {code, name, host, os}: a valid invite mints the device its own service token (added to the
//   enrolment policy) and a device token; the client writes the WARP MDM file and registers.
// - POST /v1/registered (device token): finds the registration, points "<rule prefix><name>" (a Gateway DNS
//   override) at its mesh addresses. POST /v1/leave (device token): revokes everything of that device.
// - Owner (a bearer that Cloudflare verifies as an active token of this account): POST/GET/DELETE
//   /v1/admin/invites, GET /v1/admin/devices, POST /v1/admin/devices/<name>/revoke.
// - Claude lends (docs/claude.md): the owner posts a gateway bundle (POST /v1/admin/claude); GET /claude/<code>
//   answers with claude.sh / claude.ps1 / a page, and the script claims the bundle once (POST /v1/claude/claim).
//   The bundle is stored AES-GCM sealed with a key derived from the code, which only its SHA-256 identifies.
// Invites are minted at most MINT_PER_HOUR an hour; enrolments are limited per source address; every
// state change is in the audit table.
import JOIN_SH from "./join.sh";
import LEAVE_SH from "./leave.sh";
import JOIN_PS1 from "./join.ps1";
import LEAVE_PS1 from "./leave.ps1";
import CLAUDE_SH from "./claude.sh";
import CLAUDE_PS1 from "./claude.ps1";
import CONFIG_TEXT from "./config.json"; // a text module: the instance config `mesh setup` renders

const CONFIG = JSON.parse(CONFIG_TEXT);

const API = "https://api.cloudflare.com/client/v4";
const ALPHABET = "23456789abcdefghjkmnpqrstuvwxyz"; // no 0/o, 1/l/i
const CODE_RE = /^[a-z0-9]{8}$/;
const MINT_PER_HOUR = 20;
const ENROLL_PER_HOUR = 10;
const MAX_USES = 50;
const ACTIVE = "('enrolling', 'active')";

const json = (d, status = 200, h = {}) =>
  new Response(JSON.stringify(d), { status, headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store", ...h } });
const plain = (t, type = "text/plain; charset=utf-8") => new Response(t, { headers: { "content-type": type, "cache-control": "no-store" } });
const hex = (b) => [...new Uint8Array(b)].map((x) => x.toString(16).padStart(2, "0")).join("");
const sha = async (s) => hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(s)));
const random = (n = 32) => hex(crypto.getRandomValues(new Uint8Array(n)));
const newCode = () => [...crypto.getRandomValues(new Uint8Array(8))].map((x) => ALPHABET[x % ALPHABET.length]).join("");
const clientIp = (req) => (req.headers.get("cf-connecting-ip") ?? "local").slice(0, 64);
const bearer = (req) => /^Bearer\s+(\S{1,512})\s*$/i.exec(req.headers.get("authorization") ?? "")?.[1] ?? null;
/** A DNS label: what a device is called on the mesh (<name><dns suffix>). */
export const cleanName = (s) => String(s ?? "").toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40);

/** Which bootstrap a client gets: PowerShell (its UA also says Mozilla), a browser, else the POSIX shell. */
export function scriptKind(ua) {
  const u = ua ?? "";
  if (/PowerShell/i.test(u)) return "ps1";
  if (/^Mozilla\//.test(u) && !/curl|wget/i.test(u)) return "html";
  return "sh";
}

export function commands(code = "") {
  const u = CONFIG.url.replace(/^https?:\/\//, "") + (code ? `/${code}` : "");
  const l = CONFIG.url.replace(/^https?:\/\//, "") + "/leave";
  return { sh: `curl -sL ${u}|sh`, ps1: `irm ${u}|iex`, leave_sh: `curl -sL ${l}|sh`, leave_ps1: `irm ${l}|iex` };
}

async function cf(env, method, path, body) {
  const r = await fetch(`${API}/accounts/${CONFIG.account}${path}`, {
    method,
    headers: { authorization: `Bearer ${env.CF_API_TOKEN}`, "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const d = await r.json().catch(() => ({}));
  if (r.status === 404 && method === "DELETE") return null;
  if (!r.ok || d.success === false) throw new Error(`${method} ${path.split("?")[0]}: ${r.status} ${JSON.stringify(d.errors ?? d).slice(0, 300)}`);
  return d.result;
}

const audit = (env, actor, action, detail, ip) =>
  env.DB.prepare("INSERT INTO audit (at, actor, action, detail, ip) VALUES (?, ?, ?, ?, ?)").bind(new Date().toISOString(), actor, action, detail ?? null, ip ?? null).run();

/** Seconds until key k is under n hits an hour (0: counted now). */
async function limited(env, k, n) {
  const now = Date.now();
  await env.DB.prepare("DELETE FROM hits WHERE at < ?").bind(now - 86_400_000).run();
  const r = await env.DB.prepare("SELECT COUNT(*) AS c, MIN(at) AS first FROM hits WHERE k = ? AND at > ?").bind(k, now - 3_600_000).first();
  if (r.c >= n) return Math.max(1, Math.ceil((r.first + 3_600_000 - now) / 1000));
  await env.DB.prepare("INSERT INTO hits (k, at) VALUES (?, ?)").bind(k, now).run();
  return 0;
}

const verified = new Map(); // bearer sha -> verified until (ms)
/** The owner: any active API token of this account (user- or account-owned), as Cloudflare verifies it. */
async function owner(req) {
  const t = bearer(req);
  if (!t) return null;
  const k = await sha(t);
  if ((verified.get(k) ?? 0) > Date.now()) return `owner:${k.slice(0, 8)}`;
  for (const u of [`${API}/accounts/${CONFIG.account}/tokens/verify`, `${API}/user/tokens/verify`]) {
    const d = await fetch(u, { headers: { authorization: `Bearer ${t}` } }).then((r) => r.json()).catch(() => null);
    if (d?.success && d.result?.status === "active") {
      // a user token must reach this account
      if (u.includes("/user/")) {
        const a = await fetch(`${API}/accounts/${CONFIG.account}`, { headers: { authorization: `Bearer ${t}` } }).then((r) => r.json()).catch(() => null);
        if (!a?.success) continue;
      }
      verified.set(k, Date.now() + 300_000);
      return `owner:${k.slice(0, 8)}`;
    }
  }
  return null;
}

const ruleName = (name) => `${CONFIG.rule_prefix}${name}`;
const fqdn = (name) => `${name}${CONFIG.dns_suffix}`;

async function meshRules(env) {
  const rules = (await cf(env, "GET", "/gateway/rules")) ?? [];
  return rules.filter((r) => r.name?.startsWith(CONFIG.rule_prefix));
}

async function upsertRule(env, name, ips) {
  const all = (await cf(env, "GET", "/gateway/rules")) ?? [];
  const body = {
    name: ruleName(name),
    description: `Managed by ${CONFIG.url}: mesh hostname`,
    action: "override",
    enabled: true,
    filters: ["dns"],
    traffic: `dns.fqdn in {"${name}" "${fqdn(name)}"}`,
    rule_settings: { override_ips: ips },
  };
  const old = all.find((r) => r.name === body.name);
  if (old) return (await cf(env, "PUT", `/gateway/rules/${old.id}`, { ...body, precedence: old.precedence })).id;
  const precedence = Math.max(CONFIG.rule_precedence ?? 100, ...all.map((r) => r.precedence ?? 0)) + 1;
  return (await cf(env, "POST", "/gateway/rules", { ...body, precedence })).id;
}

/** The enrolment policy admits exactly the live devices' service tokens (plus the placeholder, so never empty). */
async function syncPolicy(env) {
  const ids = (await env.DB.prepare(`SELECT service_token_id FROM devices WHERE state IN ${ACTIVE} AND service_token_id IS NOT NULL`).all()).results.map((r) => r.service_token_id);
  const include = [env.SENTINEL_TOKEN_ID, ...ids].map((token_id) => ({ service_token: { token_id } }));
  await cf(env, "PUT", `/access/policies/${env.ENROL_POLICY_ID}`, { name: CONFIG.policy_name, decision: "non_identity", include, exclude: [], require: [] });
}

/** Everything one enrolled device holds: registration, DNS rule (unless kept for a re-join), service token. */
async function revokeDevice(env, d, state, keepRule = false) {
  if (d.registration_id) {
    await cf(env, "POST", `/devices/registrations/revoke?id=${d.registration_id}`).catch((e) => console.log(String(e)));
    await cf(env, "DELETE", `/devices/registrations/${d.registration_id}`).catch((e) => console.log(String(e)));
  }
  if (!keepRule) {
    const rule = d.rule_id ? { id: d.rule_id } : (await meshRules(env)).find((r) => r.name === ruleName(d.name));
    if (rule) await cf(env, "DELETE", `/gateway/rules/${rule.id}`);
  }
  // out of the enrolment policy first: Access refuses to delete a token a policy still names
  await env.DB.prepare("UPDATE devices SET state = ?, updated = ? WHERE name = ?").bind(state, new Date().toISOString(), d.name).run();
  await syncPolicy(env);
  if (d.service_token_id) await cf(env, "DELETE", `/access/service_tokens/${d.service_token_id}`, undefined).catch((e) => console.log(String(e)));
}

async function connectorsNamed(env, name) {
  const list = (await cf(env, "GET", "/warp_connector?is_deleted=false&per_page=100").catch(() => [])) ?? [];
  return list.filter((c) => c.name === name);
}

/** Revoke a name: the device this Worker enrolled, else a WARP connector host of that name; and its DNS rule. */
async function revokeName(env, name, actor, ip, { keepRule = false } = {}) {
  const done = [];
  for (const d of (await env.DB.prepare(`SELECT * FROM devices WHERE name = ? AND state IN ${ACTIVE}`).bind(name).all()).results) {
    await revokeDevice(env, d, "revoked", keepRule);
    done.push("device");
  }
  for (const c of await connectorsNamed(env, name)) {
    await cf(env, "DELETE", `/warp_connector/${c.id}`);
    done.push("connector");
  }
  if (!keepRule) {
    for (const r of (await meshRules(env)).filter((r) => r.name === ruleName(name))) {
      await cf(env, "DELETE", `/gateway/rules/${r.id}`);
      done.push("dns");
    }
  }
  await audit(env, actor, "revoke", `${name}: ${done.join(", ") || "nothing"}`, ip);
  return done;
}

async function nameTaken(env, name) {
  if (await env.DB.prepare(`SELECT 1 AS x FROM devices WHERE name = ? AND state IN ${ACTIVE}`).bind(name).first()) return true;
  if ((await connectorsNamed(env, name)).length) return true;
  return (await meshRules(env)).some((r) => r.name === ruleName(name));
}

async function enroll(req, env) {
  const ip = clientIp(req);
  let b;
  try {
    b = await req.json();
  } catch {
    return json({ error: "JSON body required" }, 400);
  }
  const wait = await limited(env, `enroll:${ip}`, ENROLL_PER_HOUR);
  if (wait) return json({ error: "too many joins from this address; try again later", retry_after_s: wait }, 429, { "retry-after": String(wait) });
  const code = String(b.code ?? "").trim().toLowerCase();
  const now = new Date().toISOString();
  const inv = CODE_RE.test(code)
    ? await env.DB.prepare("UPDATE invites SET uses_left = uses_left - 1 WHERE code_sha256 = ? AND uses_left > 0 AND expires > ? RETURNING label, name").bind(await sha(code), now).first()
    : null;
  if (!inv) return json({ error: "this invite code is unknown, used up or expired; ask the mesh owner for a new one" }, 403);
  const codeSha = await sha(code);
  const refund = () => env.DB.prepare("UPDATE invites SET uses_left = uses_left + 1 WHERE code_sha256 = ?").bind(codeSha).run();
  const host = String(b.host ?? "").slice(0, 100);
  const name = cleanName(inv.name || b.name || host);
  if (!name || !host) return (await refund(), json({ error: "name and host required" }, 400));
  if (inv.name) await revokeName(env, name, `invite:${inv.label}`, ip, { keepRule: true }); // the owner's re-join of that name
  else if (await nameTaken(env, name)) {
    await refund();
    return json({ error: `the name ${name} is taken on this mesh; re-run with MESH_NAME=<other name>, or ask for an invite bound to it` }, 409);
  }
  const tok = await cf(env, "POST", "/access/service_tokens", { name: `${CONFIG.name}-mesh-${name}`, duration: CONFIG.token_duration });
  const deviceToken = random();
  await env.DB.prepare(
    `INSERT INTO devices (name, host, os, token_sha256, service_token_id, bound, state, ip, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'enrolling', ?7, ?8, ?8)
     ON CONFLICT(name) DO UPDATE SET host = ?2, os = ?3, token_sha256 = ?4, service_token_id = ?5, bound = ?6, state = 'enrolling', ip = ?7,
       registration_id = NULL, rule_id = NULL, ipv4 = NULL, ipv6 = NULL, created = ?8, updated = ?8`,
  )
    .bind(name, host, String(b.os ?? "").slice(0, 40), await sha(deviceToken), tok.id, inv.name ? 1 : 0, ip, now)
    .run();
  await syncPolicy(env);
  await audit(env, `invite:${inv.label}`, "enroll", `${name} (${host}, ${b.os ?? "?"})`, ip);
  const dev = CONFIG.devices?.[name] ?? {};
  return json({
    name,
    org: CONFIG.org,
    client_id: tok.client_id,
    client_secret: tok.client_secret,
    device_token: deviceToken,
    fqdn: fqdn(name),
    ssh_user: dev.ssh_user ?? "",
    leave: commands().leave_sh,
  });
}

async function deviceOf(req, env) {
  const t = bearer(req);
  return t ? await env.DB.prepare(`SELECT * FROM devices WHERE token_sha256 = ? AND state IN ${ACTIVE}`).bind(await sha(t)).first() : null;
}

async function registered(req, env) {
  const d = await deviceOf(req, env);
  if (!d) return json({ error: "unknown device token" }, 401);
  const since = new Date(Date.parse(d.created) - 600_000).toISOString();
  const regs = (await cf(env, "GET", "/devices/registrations?per_page=100&sort_by=created_at&sort_order=desc")) ?? [];
  const mine = regs.filter((r) => (r.device?.name ?? "").toLowerCase() === d.host.toLowerCase() && (r.created_at ?? "") >= since);
  const reg = mine.find((r) => /^non_identity@/.test(r.user?.email ?? "")) ?? mine[0];
  if (!reg) return json({ error: "not registered yet", retry: true }, 404);
  const ips = [reg.virtual_ipv4, reg.virtual_ipv6].filter(Boolean);
  if (!ips.length) return json({ error: "registered without mesh addresses yet", retry: true }, 404);
  const ruleId = await upsertRule(env, d.name, ips);
  if (d.bound) for (const c of await connectorsNamed(env, d.name)) await cf(env, "DELETE", `/warp_connector/${c.id}`); // its old enrolment
  await env.DB.prepare("UPDATE devices SET registration_id = ?, rule_id = ?, ipv4 = ?, ipv6 = ?, state = 'active', updated = ? WHERE name = ?")
    .bind(reg.id, ruleId, reg.virtual_ipv4 ?? null, reg.virtual_ipv6 ?? null, new Date().toISOString(), d.name)
    .run();
  await audit(env, `device:${d.name}`, "registered", ips.join(" "), clientIp(req));
  return json({ name: d.name, fqdn: fqdn(d.name), ipv4: reg.virtual_ipv4 ?? null, ipv6: reg.virtual_ipv6 ?? null });
}

async function leave(req, env) {
  const d = await deviceOf(req, env);
  if (!d) return json({ error: "unknown device token (already left or revoked?)" }, 401);
  await revokeDevice(env, d, "left");
  await audit(env, `device:${d.name}`, "leave", d.host, clientIp(req));
  return json({ ok: true, name: d.name });
}

// ---- Claude lends: one sealed gateway bundle per name, claimed once by its code
const LEND_RE = /^[a-z0-9-]{1,40}$/;
const LEND_CODE_RE = /^[a-z0-9]{12}$/;
const SAFE = /^[\w.~:/?#@&+,;=%\- ]{1,2048}$/; // what the scripts may embed without quoting: no quotes, $, ` or \
const CLAIM_PER_HOUR = 10;
const sealKey = async (code) =>
  crypto.subtle.importKey("raw", await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`cx-claude-seal:${code}`)), "AES-GCM", false, ["encrypt", "decrypt"]);
const b64 = (b) => btoa(String.fromCharCode(...new Uint8Array(b)));
const unb64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

export function claudeCommands(code) {
  const u = `${CONFIG.url.replace(/^https?:\/\//, "")}/claude/${code}`;
  return { sh: `curl -sL ${u}|sh`, ps1: `irm ${u}|iex` };
}

/** A lend bundle as the owner sends it: {base_url, token, headers[]}, every value plain enough to embed. */
function checkBundle(b) {
  const out = { base_url: String(b?.base_url ?? ""), token: String(b?.token ?? ""), headers: (b?.headers ?? []).map(String) };
  if (!/^https:\/\//.test(out.base_url) && out.base_url) throw new Error("base_url must be https");
  if (out.base_url && !SAFE.test(out.base_url)) throw new Error("bundle values must be plain text");
  for (const v of [out.token, ...out.headers]) if (!SAFE.test(v)) throw new Error("bundle values must be plain text");
  if (out.headers.length > 8) throw new Error("too many headers");
  return out;
}

async function lendAdmin(req, env, p, actor, ip, now) {
  if (p === "/v1/admin/claude" && req.method === "POST") {
    const b = await req.json().catch(() => ({}));
    const name = String(b.name ?? "");
    if (!LEND_RE.test(name)) return json({ error: "name: lowercase letters, digits and -" }, 400);
    let bundle;
    try {
      bundle = checkBundle(b.bundle);
    } catch (e) {
      return json({ error: e.message }, 400);
    }
    const mesh = String(b.mesh_url ?? "");
    if (mesh && !/^https?:\/\//.test(mesh)) return json({ error: "mesh_url must be http(s)" }, 400);
    if (mesh && !SAFE.test(mesh)) return json({ error: "mesh_url must be plain text" }, 400);
    if (!bundle.base_url && !mesh) return json({ error: "a lend needs base_url or mesh_url" }, 400);
    const wait = await limited(env, "mint", MINT_PER_HOUR);
    if (wait) return json({ error: "too many codes this hour", retry_after_s: wait }, 429);
    const code = [...crypto.getRandomValues(new Uint8Array(12))].map((x) => ALPHABET[x % ALPHABET.length]).join("");
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, await sealKey(code), new TextEncoder().encode(JSON.stringify(bundle)));
    const hours = Math.min(24 * 7, Math.max(0.1, Number(b.code_hours ?? 24)));
    const codeExpires = new Date(now.getTime() + hours * 3_600_000).toISOString();
    const expires = b.expires ? new Date(b.expires).toISOString() : null;
    await env.DB.prepare(
      `INSERT INTO lends (name, code_sha256, sealed, iv, mesh_url, mesh_only, code_expires, expires, state, created) VALUES (?1, ?2, ?3, ?4, ?5, ?9, ?6, ?7, 'pending', ?8)
       ON CONFLICT(name) DO UPDATE SET code_sha256 = ?2, sealed = ?3, iv = ?4, mesh_url = ?5, mesh_only = ?9, code_expires = ?6, expires = ?7, state = 'pending', created = ?8, claimed = NULL, claimed_ip = NULL`,
    )
      .bind(name, await sha(code), b64(sealed), b64(iv), mesh || null, codeExpires, expires, now.toISOString(), bundle.base_url ? 0 : 1)
      .run();
    await audit(env, actor, "lend", `${name} code until ${codeExpires}${mesh ? " mesh" : ""}${bundle.base_url ? "" : " mesh-only"}`, ip);
    return json({ name, code, code_expires: codeExpires, expires, ...claudeCommands(code) });
  }
  if (p === "/v1/admin/claude" && req.method === "GET") {
    const lends = (await env.DB.prepare("SELECT name, state, mesh_url, mesh_only, code_expires, expires, created, claimed, claimed_ip FROM lends ORDER BY name").all()).results;
    for (const l of lends) if (l.state === "pending" && l.code_expires <= now.toISOString()) l.state = "code expired";
    return json({ lends });
  }
  const m = /^\/v1\/admin\/claude\/([a-z0-9-]{1,40})\/revoke$/.exec(p);
  if (m && req.method === "POST") {
    const r = await env.DB.prepare("UPDATE lends SET state = 'revoked', sealed = NULL, iv = NULL, code_sha256 = NULL WHERE name = ? AND state != 'revoked'").bind(m[1]).run();
    await audit(env, actor, "lend-revoke", m[1], ip);
    return json({ name: m[1], revoked: (r.meta?.changes ?? 0) > 0 });
  }
  return null;
}

/** The lend a code names, while it can still be claimed. */
const lendOf = async (env, code) =>
  LEND_CODE_RE.test(code)
    ? await env.DB.prepare("SELECT * FROM lends WHERE code_sha256 = ? AND state = 'pending' AND sealed IS NOT NULL AND code_expires > ?").bind(await sha(code), new Date().toISOString()).first()
    : null;

async function claim(req, env) {
  const ip = clientIp(req);
  const b = await req.json().catch(() => ({}));
  const wait = await limited(env, `claim:${ip}`, CLAIM_PER_HOUR);
  if (wait) return json({ error: "too many claims from this address; try again later", retry_after_s: wait }, 429, { "retry-after": String(wait) });
  const code = String(b.code ?? "").trim().toLowerCase();
  const l = await lendOf(env, code);
  // take it: only one claim's UPDATE matches (RETURNING would give the nulled columns, so read first)
  const took = l && (await env.DB.prepare("UPDATE lends SET state = 'claimed', claimed = ?2, claimed_ip = ?3, sealed = NULL, iv = NULL, code_sha256 = NULL WHERE code_sha256 = ?1 AND state = 'pending'")
    .bind(l.code_sha256, new Date().toISOString(), ip).run()).meta?.changes === 1;
  if (!took) return json({ error: "this code is unknown, already used or expired; ask for a new one" }, 403);
  const plainBytes = await crypto.subtle.decrypt({ name: "AES-GCM", iv: unb64(l.iv) }, await sealKey(code), unb64(l.sealed));
  const bundle = JSON.parse(new TextDecoder().decode(plainBytes));
  await audit(env, `lend:${l.name}`, "claim", null, ip);
  const out = { name: l.name, mesh_url: l.mesh_url ?? "", ...bundle };
  if (b.format !== "lines") return json(out);
  const lines = [`name=${out.name}`, `base_url=${out.base_url}`, `mesh_url=${out.mesh_url}`, `token=${out.token}`, ...out.headers.map((h) => `header=${h}`)];
  return plain(lines.join("\n") + "\n");
}

async function claudeScript(env, kind, code) {
  const l = await lendOf(env, code);
  if (kind === "html") return claudePage(code, !!l);
  if (!l) {
    const msg = "this Claude code is unknown, already used or expired; ask for a new one";
    return plain(kind === "ps1" ? `Write-Error '${msg}'; return\n` : `#!/bin/sh\necho '[claude] error: ${msg}' >&2; exit 1\n`);
  }
  const body = (kind === "ps1" ? CLAUDE_PS1 : CLAUDE_SH)
    .replaceAll("__MESH_URL__", CONFIG.url)
    .replaceAll("__CLAUDE_CODE__", code)
    .replaceAll("__CLAUDE_MESH_URL__", l.mesh_url ?? "")
    .replaceAll("__CLAUDE_MESH_ONLY__", l.mesh_only ? "1" : "");
  return plain(body);
}

function claudePage(code, live) {
  const c = claudeCommands(code);
  const esc = (s) => String(s).replace(/[&<>"]/g, (m) => `&#${m.charCodeAt(0)};`);
  return plain(
    `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Claude Code setup</title>
<style>:root{--bg:#f7f6f2;--fg:#1d1c1a;--mut:#6b6862;--box:#ecebe5}@media (prefers-color-scheme:dark){:root{--bg:#141413;--fg:#ecebe5;--mut:#9c9a93;--box:#23221f}}
body{background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif;max-width:44rem;margin:3rem auto;padding:0 16px}code{display:block;background:var(--box);padding:.7rem 1rem;border-radius:8px;overflow-x:auto;font-size:15px;-webkit-user-select:all;user-select:all}p{color:var(--mut)}</style>
<h1>Set up Claude Code</h1>${live ? "" : "<p>This code is unknown, already used or expired: ask for a new one.</p>"}
<p>Run this once on the machine that runs Claude Code. Linux and macOS, in a terminal:</p><code>${esc(c.sh)}</code>
<p>Windows, in PowerShell:</p><code>${esc(c.ps1)}</code>
<p>It points Claude Code at the shared gateway in your settings.json (merged, with a backup) and checks that it answers. The code works once.</p>`,
    "text/html; charset=utf-8",
  );
}

async function admin(req, env, p) {
  const actor = await owner(req);
  if (!actor) return json({ error: "an active API token of this Cloudflare account is required" }, 401);
  const ip = clientIp(req);
  const now = new Date();
  if (p === "/v1/admin/invites" && req.method === "POST") {
    const b = await req.json().catch(() => ({}));
    const wait = await limited(env, "mint", MINT_PER_HOUR);
    if (wait) return json({ error: "too many invites this hour", retry_after_s: wait }, 429);
    const uses = Math.min(MAX_USES, Math.max(1, Number(b.uses ?? 1) | 0));
    const hours = Math.min(24 * 30, Math.max(0.1, Number(b.hours ?? CONFIG.invite_ttl_hours ?? 24)));
    const name = b.name ? cleanName(b.name) : null;
    const code = newCode();
    const label = `${now.toISOString().slice(0, 16)}-${random(2)}`;
    const expires = new Date(now.getTime() + hours * 3_600_000).toISOString();
    await env.DB.prepare("INSERT INTO invites (code_sha256, label, name, uses_left, expires, created) VALUES (?, ?, ?, ?, ?, ?)").bind(await sha(code), label, name, uses, expires, now.toISOString()).run();
    await audit(env, actor, "invite", `${label} uses=${uses} hours=${hours}${name ? ` name=${name}` : ""}`, ip);
    return json({ code, label, name, uses, expires, ...commands(code) });
  }
  if (p === "/v1/admin/invites" && req.method === "GET")
    return json({ invites: (await env.DB.prepare("SELECT label, name, uses_left, expires, created FROM invites WHERE uses_left > 0 AND expires > ? ORDER BY created DESC").bind(now.toISOString()).all()).results });
  if (p === "/v1/admin/invites" && req.method === "DELETE") {
    const r = await env.DB.prepare("UPDATE invites SET uses_left = 0 WHERE uses_left > 0").run();
    await audit(env, actor, "revoke-invites", String(r.meta?.changes ?? ""), ip);
    return json({ ok: true, revoked: r.meta?.changes ?? 0 });
  }
  if (p === "/v1/admin/devices" && req.method === "GET") {
    const enrolled = (await env.DB.prepare("SELECT name, host, os, ipv4, ipv6, state, created, updated FROM devices ORDER BY name").all()).results;
    const connectors = ((await cf(env, "GET", "/warp_connector?is_deleted=false&per_page=100").catch(() => [])) ?? []).map((c) => ({ name: c.name, status: c.status, id: c.id }));
    const dns = (await meshRules(env)).map((r) => ({ name: r.name.slice(CONFIG.rule_prefix.length), ips: r.rule_settings?.override_ips ?? [] }));
    return json({ dns_suffix: CONFIG.dns_suffix, enrolled, connectors, dns });
  }
  if (p === "/v1/admin/audit" && req.method === "GET")
    return json({ audit: (await env.DB.prepare("SELECT at, actor, action, detail, ip FROM audit ORDER BY id DESC LIMIT 200").all()).results });
  if (p.startsWith("/v1/admin/claude")) {
    const r = await lendAdmin(req, env, p, actor, ip, now);
    if (r) return r;
  }
  const m = /^\/v1\/admin\/devices\/([a-z0-9-]{1,40})\/revoke$/.exec(p);
  if (m && req.method === "POST") return json({ name: m[1], revoked: await revokeName(env, m[1], actor, ip) });
  return json({ error: "not found" }, 404);
}

function page(code) {
  const c = commands(code);
  const esc = (s) => String(s).replace(/[&<>"]/g, (m) => `&#${m.charCodeAt(0)};`);
  return plain(
    `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Join the mesh</title>
<style>:root{--bg:#f7f6f2;--fg:#1d1c1a;--mut:#6b6862;--box:#ecebe5}@media (prefers-color-scheme:dark){:root{--bg:#141413;--fg:#ecebe5;--mut:#9c9a93;--box:#23221f}}
body{background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif;max-width:44rem;margin:3rem auto;padding:0 16px}code{display:block;background:var(--box);padding:.7rem 1rem;border-radius:8px;overflow-x:auto;font-size:15px;-webkit-user-select:all;user-select:all}p{color:var(--mut)}</style>
<h1>Join the ${esc(CONFIG.name)} mesh</h1>${code ? "" : "<p>You need an invite code from the mesh owner: the command they send you ends in it.</p>"}
<p>Linux and macOS, in a terminal:</p><code>${esc(c.sh)}</code><p>Windows, in an administrator PowerShell:</p><code>${esc(c.ps1)}</code>
<p>It installs Cloudflare WARP if needed and enrols this machine: internet through WARP, your local networks untouched, and the mesh reachable. To leave:</p>
<code>${esc(c.leave_sh)}</code><code>${esc(c.leave_ps1)}</code>`,
    "text/html; charset=utf-8",
  );
}

function script(kind, leaving, code) {
  const body = (kind === "ps1" ? (leaving ? LEAVE_PS1 : JOIN_PS1) : leaving ? LEAVE_SH : JOIN_SH)
    .replaceAll("__MESH_URL__", CONFIG.url)
    .replaceAll("__MESH_CODE__", code)
    .replaceAll("__MESH_NAME__", CONFIG.name)
    .replaceAll("__MESH_TAILSCALE_RULE__", String(CONFIG.tailscale_rule_priority ?? 5200));
  return plain(body);
}

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    const p = url.pathname;
    try {
      if (req.method === "GET" || req.method === "HEAD") {
        const kind = scriptKind(req.headers.get("user-agent"));
        if (p === "/" || CODE_RE.test(p.slice(1).toLowerCase())) {
          const code = p === "/" ? "" : p.slice(1).toLowerCase();
          return kind === "html" ? page(code) : script(kind, false, code);
        }
        if (p === "/leave") return kind === "html" ? page("") : script(kind, true, "");
        const lc = /^\/claude\/([a-z0-9]{1,40})$/i.exec(p);
        if (lc) return await claudeScript(env, kind, lc[1].toLowerCase());
        if (p === "/v1/keys") return plain((CONFIG.ssh_keys ?? []).join("\n") + "\n");
      }
      if (req.method === "POST" && p === "/v1/enroll") return await enroll(req, env);
      if (req.method === "POST" && p === "/v1/registered") return await registered(req, env);
      if (req.method === "POST" && p === "/v1/claude/claim") return await claim(req, env);
      if (req.method === "POST" && p === "/v1/leave") return await leave(req, env);
      if (p.startsWith("/v1/admin/")) return await admin(req, env, p);
      return json({ error: "not found" }, 404);
    } catch (e) {
      console.log(String(e?.stack ?? e));
      return json({ error: "the mesh endpoint failed; try again, or tell the mesh owner", detail: String(e?.message ?? e).slice(0, 300) }, 502);
    }
  },
};
