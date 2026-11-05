// The mesh Worker (mesh/worker/worker.js) served over local HTTPS against an in-memory D1 (node:sqlite) and a
// fake Cloudflare API that accepts one owner token. tests/mesh_ssh_overlay.py drives `cx mesh invite --short` and
// `cx join <host>/<code>` through mesh.py against it, end to end. No account is touched; it listens on 127.0.0.1 only.
// Run: node tests/mesh_worker_server.mjs CERT KEY PORT OWNER_TOKEN   (prints "ready" once listening)
import { DatabaseSync } from "node:sqlite";
import { readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { createServer } from "node:https";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const [cert, key, port, OWNER] = process.argv.slice(2);
const W = join(dirname(fileURLToPath(import.meta.url)), "..", "mesh", "worker");
const CONFIG = {
  name: "demo", account: "acct", org: "demo-team", url: `https://localhost:${port}`, dns_suffix: ".mesh.example.org",
  rule_prefix: "Mesh DNS: ", policy_name: "demo-mesh enrolment", token_duration: "8760h", invite_ttl_hours: 24,
  devices: {}, ssh_keys: [], tailscale_rule_priority: 5200,
};
const text = (f) => JSON.stringify(readFileSync(join(W, f), "utf8"));
const src = readFileSync(join(W, "worker.js"), "utf8")
  .replace(/^import (\w+) from "\.\/(join|leave|claude)\.(sh|ps1)";$/gm, (_, v, n, e) => `const ${v} = ${text(`${n}.${e}`)};`)
  .replace(/^import CONFIG_TEXT from "\.\/config\.json";.*$/m, `const CONFIG_TEXT = ${JSON.stringify(JSON.stringify(CONFIG))};`);
const file = join(mkdtempSync(join(tmpdir(), "mesh-worker-")), "worker.mjs");
writeFileSync(file, src);
const worker = await import(file);

const db = new DatabaseSync(":memory:");
db.exec(readFileSync(join(W, "schema.sql"), "utf8"));
const stmt = (sql, args = []) => ({
  bind: (...a) => stmt(sql, a),
  run: async () => ({ meta: { changes: Number(db.prepare(sql).run(...args).changes) } }),
  first: async () => db.prepare(sql).get(...args) ?? null,
  all: async () => ({ results: db.prepare(sql).all(...args) }),
});
const env = { DB: { prepare: (sql) => stmt(sql) }, CF_API_TOKEN: "worker-token", ENROL_POLICY_ID: "pol", SENTINEL_TOKEN_ID: "sentinel" };

// The Cloudflare API, as far as owner authentication needs it.
globalThis.fetch = async (url, init = {}) => {
  const auth = (init.headers?.authorization ?? "").replace("Bearer ", "");
  const ok = auth === OWNER && /\/tokens\/verify$|\/accounts\/acct$/.test(new URL(url).pathname);
  return new Response(JSON.stringify(ok ? { success: true, result: { status: "active" } } : { success: false }), { status: ok ? 200 : 401 });
};

createServer({ cert: readFileSync(cert), key: readFileSync(key) }, async (req, res) => {
  const chunks = [];
  for await (const c of req) chunks.push(c);
  const body = chunks.length ? Buffer.concat(chunks) : undefined;
  const headers = { ...req.headers, "cf-connecting-ip": req.socket.remoteAddress };
  const r = await worker.default.fetch(new Request(`https://localhost:${port}${req.url}`, { method: req.method, headers, body }), env);
  res.writeHead(r.status, Object.fromEntries(r.headers));
  res.end(Buffer.from(await r.arrayBuffer()));
}).listen(Number(port), "127.0.0.1", () => console.log("ready"));
