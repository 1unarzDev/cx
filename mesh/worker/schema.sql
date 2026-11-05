-- The share-code Worker's state (worker.js): invites, the devices it enrolled, rate-limit hits and an audit log.
CREATE TABLE IF NOT EXISTS invites (
  code_sha256 TEXT PRIMARY KEY,
  label TEXT NOT NULL,
  name TEXT,                 -- a bound device name (a re-join replaces that device), else the joiner's hostname
  uses_left INTEGER NOT NULL,
  expires TEXT NOT NULL,
  created TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS devices (
  name TEXT PRIMARY KEY,
  host TEXT NOT NULL,        -- the hostname WARP registers under
  os TEXT,
  token_sha256 TEXT NOT NULL UNIQUE,
  service_token_id TEXT,
  registration_id TEXT,
  rule_id TEXT,
  ipv4 TEXT,
  ipv6 TEXT,
  bound INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL,       -- enrolling | active | left | revoked
  ip TEXT,
  created TEXT NOT NULL,
  updated TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS hits (k TEXT NOT NULL, at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS hits_k ON hits (k, at);
CREATE TABLE IF NOT EXISTS audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at TEXT NOT NULL,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  detail TEXT,
  ip TEXT
);
CREATE TABLE IF NOT EXISTS lends (  -- Claude gateway lends (docs/claude.md), one per name
  name TEXT PRIMARY KEY,
  code_sha256 TEXT UNIQUE,   -- while pending: the claim code's SHA-256
  sealed TEXT,               -- while pending: the bundle, AES-GCM under a key derived from the code
  iv TEXT,
  mesh_url TEXT,             -- the mesh-only gateway, when there is one (not secret)
  mesh_only INTEGER NOT NULL DEFAULT 0, -- no Access route: the mesh is required
  code_expires TEXT,
  expires TEXT,              -- when the lend itself ends (the owner revokes it then)
  state TEXT NOT NULL,       -- pending | claimed | revoked
  created TEXT NOT NULL,
  claimed TEXT,
  claimed_ip TEXT
);
CREATE TABLE IF NOT EXISTS pairs (  -- ssh-overlay short codes (docs/mesh.md, "cx mesh pair"): a sealed direct code
  lookup TEXT PRIMARY KEY,   -- SHA-256 of a derivation of the short code (the Worker never sees the code)
  sealed TEXT,               -- sealed by cx under a key from the short code; NULL once redeemed
  expires TEXT NOT NULL,
  state TEXT NOT NULL,       -- pending | redeemed
  created TEXT NOT NULL,
  redeemed TEXT,
  redeemed_ip TEXT
);
