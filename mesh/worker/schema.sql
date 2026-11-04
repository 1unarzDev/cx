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
