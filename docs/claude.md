# Claude: lend your Claude Code gateway with one line

`cx claude` hands a teammate your Claude Code gateway (a [better-ccflare](https://github.com/tombii/better-ccflare)
behind Cloudflare Access, optionally also on the mesh) in one step. It needs a `cx mesh` (docs/mesh.md): the
mesh's Worker serves the codes.

```sh
# owner
cx claude lend evan --days 30        # a gateway key, an Access service token and its policy, and a code (--hours H)
cx claude list                       # lends: pending, claimed or revoked, key state, expiry
cx claude revoke evan                # policy, service token, the key (disabled), a pending code
cx claude revoke --expired           # every lend past its expiry

# the teammate, no cx needed
curl -sL mesh.example.org/claude/<code>|sh     # Linux, macOS
irm mesh.example.org/claude/<code>|iex         # Windows PowerShell
```

`lend` prints the two one-liners. The script writes `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN` and, off the
mesh, `ANTHROPIC_CUSTOM_HEADERS` (the CF-Access headers) into the `env` of `~/.claude/settings.json`
(`$CLAUDE_CONFIG_DIR` when set). It merges them into what is there, after a timestamped backup, then sends one free
request (token counting) and prints `ready`. Every command takes `--json` and `--config PATH`.

## Config

Add to the mesh config (`~/.config/cx/mesh/config.toml`):

```toml
[claude]
gateway = ["ssh", "gw-host", "docker", "exec", "better-ccflare", "better-ccflare"]  # runs the better-ccflare CLI
base_url = "https://claude.example.org"   # the gateway behind Access; the Access app on this host admits the lends
mesh_url = "http://100.96.0.5:8081"       # optional: the gateway on a mesh address, no Access token needed
# access_app = "<id or domain>"  prefix = "cc-lend-"  days = 30  code_hours = 24
```

The Worker needs no new settings; after updating cx, run `cx mesh deploy` once so it serves `/claude/`.

The Cloudflare API token (`$CLOUDFLARE_API_TOKEN`, the credentials file, or cx's stored `cloudflare.api_token`, see
[secrets.md](secrets.md#meshpy)) is used by claude.py for Access; the `gateway` command runs with
`$CLOUDFLARE_API_TOKEN` removed from its environment, so it never receives the admin token.

## What a lend is

Everything is named `<prefix><name>`, so `revoke` finds it again without local state:

- **A gateway key**, through better-ccflare's own CLI (`--generate-api-key`). The CLI shows a key only when it
  makes one, so a new lend of a name replaces the old one.
- **An Access service token**, which lasts as long as the lend, and **an app policy** (`non_identity`, that token
  only) on the gateway's Access app. `--mesh-only` skips both.
- **A code**: 12 characters, single use, `code_hours` long. The Worker keeps the bundle sealed (AES-GCM, with a key
  derived from the code) and stores only the code's SHA-256. Opening the link (`GET /claude/<code>`) returns the
  script and claims nothing, so a chat preview cannot use it up. The script claims the bundle once over HTTPS
  (`POST /v1/claude/claim`) and the Worker then deletes it. Claims are rate limited per address.

**The route:** when `mesh_url` answers `/health` from the teammate's machine, the script uses it and writes no Access
headers. Otherwise it uses `base_url` with the headers. A `--mesh-only` lend that cannot reach the mesh stops before
it claims, so the code still works once the machine joins. `CLAUDE_LEND_ROUTE=mesh|access` forces a route.

On the mesh, the gateway key is the only check. It stays valid until `revoke`, so run `cx claude revoke --expired`
from time to time. Off the mesh, Access also stops at the token's expiry.

The secrets never appear in logs or argv: the script keeps them in mode-600 temporary files and its curl config.

## Tests

`python3 tests/mesh_util.py` covers lend, replace, revoke, a failed lend that undoes itself and `--expired`, against
a fake Cloudflare and a fake better-ccflare. It also runs `claude.sh` against a local stand-in for the Worker and the
gateway: the merge, the backup, mode 600, both routes, a single use, and no secret in its output.
`node tests/mesh_worker.mjs` covers the sealed storage, the served scripts, the single claim and revoke.
