# Mesh: one-line joins on your own Cloudflare account

`cx mesh` builds a private mesh on a free Cloudflare One (Zero Trust) account you own, optionally on a domain you
own. One device profile applies everywhere:

- **Internet** goes through WARP over MASQUE.
- **LANs** stay local: RFC 1918, link-local, multicast (so mDNS keeps working), loopback and the rest of CGNAT are
  split-tunnel excludes, plus any LANs you add.
- **The mesh range** (`100.96.0.0/12`, WARP's virtual IPs) is carved out of those excludes, so it rides the tunnel.
  Every device gets `<name><dns suffix>` for ssh.

A device joins with one line and needs no cx. The line installs WARP when it is missing (the Cloudflare apt or dnf
repository with its keyring, `cloudflare-warp-bin` from the AUR via paru, yay or makepkg, the macOS pkg or Homebrew,
the Windows msi or winget). It then enrols the device, connects at boot and prints the ssh line. Leaving restores
what was there before.

```sh
# owner, once
cx mesh setup --name team --domain example.org     # --domain is optional: without one, <name>-mesh.<you>.workers.dev
cx mesh invite                                     # a code good for one join within 24 h (--uses N, --hours H, --name DEVICE)

# the device: Linux or macOS
curl -sL mesh.example.org/<code>|sh
# the device: Windows, in an administrator PowerShell
irm mesh.example.org/<code>|iex

cx mesh devices                                    # joined devices, WARP connectors, mesh names
cx mesh revoke laptop                              # its registration, service token and name
curl -sL mesh.example.org/leave|sh                 # on the device: leave and restore
cx mesh teardown                                   # removes everything setup created
```

Every command takes `--json` and `--config PATH`. `setup --dry-run` reads the account and lists the changes
without making any. Opening the endpoint in a browser shows the same commands.

## Without cx

`mesh/` is self-contained: it needs only `python3` (standard library) on the owner's machine and `curl` and `sh` on a
device.

```sh
curl -fsSL https://github.com/1unarzDev/cx/archive/refs/heads/main.tar.gz | tar -xz --strip-components=1 cx-main/mesh
python3 mesh/mesh.py setup --name team
```

`cx mesh` runs the same util from a copy embedded in the cx binary.

## Setup token

Create an API token at dash.cloudflare.com, under *My Profile, API Tokens*, scoped to your account. Setup reads it
from `$CLOUDFLARE_API_TOKEN`, or asks once and saves it (mode 600) next to the config. Under `cx mesh`, a token kept in
cx's secrets store (`cx secrets set cloudflare.api_token`, or `cx secrets migrate` to import the saved file; see
[secrets.md](secrets.md)) is passed in as `$CLOUDFLARE_API_TOKEN` for the subcommands that call the API (never `join` or
`leave`, whose fetched script also runs with the variable removed, and never with `--config`, whose mesh keeps the
`credentials` file next to it), so the default mesh's plaintext file can go. If that stored token cannot be read in this
session (it is in the OS keyring and there is no session bus), `cx mesh` stops instead of letting setup ask again and
save it in plaintext. First-run `cx mesh setup` still saves the file; `cx secrets migrate` imports it. The
cloudflare-warp backend (`--backend cloudflare-warp`) gets the stored token the same way. `mesh.py token-url`
(`cx mesh token-url`) prints a link that opens the form with most permissions already filled in. It needs these
permissions:

- Account: Zero Trust Write, Access: Apps and Policies Write, Access: Service Tokens Write, Access: Organizations
  Read, Cloudflare One Connector: WARP Write, D1 Write, Workers Scripts Write, Account API Tokens Write.
- Zone, only with a domain: Workers Routes Write and Zone Read.

Setup gives the Worker its own narrower token (Zero Trust, Access and WARP connectors) as a secret binding. The owner
commands authenticate to the Worker with any active token of the account, which Cloudflare verifies.

Before the first setup, open one.dash.cloudflare.com once and pick the Free plan, so the account has a Zero Trust
organization.

### Onboarding helpers

These need no config, and the token never appears in their output:

```sh
cx mesh token-url [--with-domain] [--json]       # the pre-filled "create token" link (no network)
cx mesh verify [--token-stdin] [--json]          # GET /user/tokens/verify, then /accounts/<id>/tokens/verify
cx mesh zones [--token-stdin] [--json]           # the zones the token can use, to pick --domain
```

`token-url` builds a [user API token template URL][template]:
`https://dash.cloudflare.com/profile/api-tokens?permissionGroupKeys=<url-encoded JSON>&accountId=*&zoneId=all&name=cx-mesh`.
The JSON is a list of `{"key": ..., "type": "read"|"edit"}`. The template takes `edit`, not `write`. User-token
URLs accept only the short keys and silently drop any they do not know ([cloudflare-docs#29825][keys], which
corrected `workers_kv` and `zone_dns`). The keys used, from the reference table on that page:

| Permission (above) | Key | Type |
|---|---|---|
| Zero Trust Write | `teams` | edit |
| Access: Apps and Policies Write | `access` | edit |
| Access: Organizations Read | `access_acct` | read |
| D1 Write | `d1` | edit |
| Workers Scripts Write | `workers_scripts` | edit |
| Account API Tokens Write | `account_api_tokens` | edit |
| Workers Routes Write (domain) | `workers_routes` | edit |
| Zone Read (domain) | `zone` | read |

**Access: Service Tokens Write** and **Cloudflare One Connector: WARP Write** have no template key. Account-token
URLs accept permission group IDs, but those come only from `GET /accounts/<id>/tokens/permission_groups`, which
already needs a token. So `token-url` returns them as `manual`, and the form needs them added by hand. Names are as
in the [permissions reference][perms]. Checked 2026-11-04.

[template]: https://developers.cloudflare.com/fundamentals/api/how-to/account-owned-token-template/
[keys]: https://github.com/cloudflare/cloudflare-docs/issues/29825
[perms]: https://developers.cloudflare.com/fundamentals/api/reference/permissions/

## What setup creates

All names start with the mesh name, and the state file (`~/.config/cx/mesh/<name>.state.json`) records the IDs, so
teardown removes exactly these:

- **Device settings:** virtual IPs and the Gateway TCP and UDP proxy are switched on. These are account-wide.
- **Device profile `<name>-mesh`:** MASQUE and the excludes above. It matches devices enrolled by service token
  (`non_identity@<team>`); set `match` in the config to narrow it on an account that has other service-token
  devices. WARP connectors are `warp_connector@<team>` and keep their own profile.
- **Enrolment:** the `<name>-mesh enrolment` Service Auth policy on the WARP login app. Other policies on that app
  are kept. The Worker keeps the policy's include list equal to the devices it enrolled.
- **D1 database `<name>-mesh`:** invites (only their SHA-256 is stored), devices, rate-limit hits and an audit log.
- **Worker `<name>-mesh`:** on `mesh.<domain>`, or on workers.dev.

`~/.config/cx/mesh/config.toml` holds the profile, the DNS suffix, invite lifetime, the ssh keys a join authorizes
and an optional ssh user per device. Edit it and run `setup` again to apply.

## The join

1. The device posts the code. A valid code mints that device its own service token, which is added to the
   enrolment policy, and a device token. Codes are 8 characters, single or limited use, and expire. Joins are rate
   limited per source address and invites per hour.
2. The script writes the WARP MDM file (organization, client id and secret) and restarts WARP, which registers
   and connects. It keeps the previous MDM file and the service state for leave.
3. The Worker reads the new registration's mesh addresses from Cloudflare and points a Gateway DNS override
   (`Mesh DNS: <name>`) at them.
4. **Tailscale on Linux:** WARP's firewall passes only its tunnel and the excluded ranges, which would drop
   Tailscale's own packets. The join adds an `ip rule` that sends them (fwmark `0x80000`) through WARP's table, and a
   `tailscaled` drop-in so the rule survives restarts. Tailscale drops CGNAT IPv4
   that does not arrive on `tailscale0`, so the join prints the mesh IPv6 ssh line too.
5. The owner's keys (`ssh_keys`) go into `authorized_keys`, each marked `cx-mesh`.

`MESH_NAME=<name>` picks the device's mesh name (the hostname by default). `MESH_DRY_RUN=1` prints what the join
would do. An invite with `--name DEVICE` re-enrols that name, replacing its old registration or WARP connector.

Leave deregisters the device on the server: its registration, service token and DNS name. It then removes the
MDM file or puts the previous one back, restores the service state, the Tailscale rule and the marked keys, and
uninstalls WARP when the join installed it.

## Backends

`cx mesh` is one of three mesh backends. `cx mesh backends [--json]` lists them with their status, the address
ranges each claims, and any collision between those ranges and this host's LANs. `cx mesh ranges` prints only the
ranges and collisions. Any subcommand takes `--backend ID` (`status`, `members`, `invite`, `revoke`, `audit`,
`ranges`, `setup`, `teardown`). Without `--backend`, `cx mesh` is the WARP util described above, unchanged.

| Backend | What it is | Ranges |
|---|---|---|
| `ssh-overlay` (default) | Built in: enrolled devices, automatic core pairing, the existing SSH routes and ProxyJump through a reachable peer. Coexists with any VPN. | none |
| `cloudflare-warp` | This document's WARP mesh, driven through `mesh.py --json` | `profile.mesh` (default `100.96.0.0/12`) |
| `tailscale` | Read-only adoption of `tailscale status --json` when the binary is on PATH: members, MagicDNS names | `100.64.0.0/10`, `fd7a:115c:a1e0::/48` |

A backend claims its ranges only on evidence: `cloudflare-warp` when a mesh config exists here
(`~/.config/cx/mesh/config.toml`), `tailscale` when its binary is on PATH, either one when an interface address is
attributed to it. An unconfigured host with no WARP interface makes no WARP claim. WARP's `100.96.0.0/12` lies
inside Tailscale's `100.64.0.0/10`. The report shows that as a `nested` collision when both are present. An
interface address is attributed by interface name first (`tailscale0`, `CloudflareWARP`), then by the most
specific range. The same rule fills `overlays.interfaces` in the network observation, so a `tailscale0` address
in `100.100.0.0/16` stays Tailscale's. The network observation's `overlay` field is now `observed`, `configured` or
`none`, and `overlays` carries the details. Every network observation computes it from local files and the
interface list only: it never runs `tailscale status` or takes the ssh-overlay audit lock. `cx mesh backends` is
where each backend's full status is read.

## ssh-overlay: `cx mesh pair`

```sh
# the viewer (a device already in your cx mesh); runs in the foreground until one device joins
cx mesh invite --backend ssh-overlay [--hours 24] [--name rover] [--address IP]... [--port 7241]
# the new device (cx installed; Linux), with the line the viewer printed
cx mesh pair 7K2Q-M9XD-4HT host[:port] [--name N]
```

`cx mesh join` is the WARP join above; `cx mesh pair` is this one.

**The code** is 11 Crockford base32 symbols, shown as `XXXX-XXXX-XXX`: 10 random symbols (50 bits) and a check
symbol (a weighted sum in GF(32)) that catches every single mistyped symbol and every swap of two neighbours
offline. Input ignores case, dashes and spaces; `I`/`L` read as `1` and `O` as `0`. The code carries no address:
the joiner names the viewer's host, which the viewer prints. The code and its key are never written to disk:
they live only in the memory of the `cx mesh invite` that serves them.

**The listener.** `cx mesh invite --backend ssh-overlay` opens a plain TCP listener on this host's mesh and LAN
addresses only: private IPv4, the CGNAT range overlays use (`100.64.0.0/10`: Tailscale, WARP), and IPv6 ULA
(`fc00::/7`). It never binds a public, link-local or unspecified address. `--address IP` (repeatable) picks the
addresses instead; loopback is allowed there, and anything else outside those ranges is refused. The port is
7241 unless `--port` says otherwise; the printed line names it when it is not the default. The listener runs in
the foreground of `cx mesh invite`, so nothing is left running in the background. It closes after one join, at
expiry (24 hours by default, at most 7 days), on Ctrl-C, or when the invite is cancelled. Connections are served
one at a time. Each request is one CX1 frame (`CX1 <len>` plus JSON, at most 64 KiB), read within 10 seconds
however slowly it arrives.

**The protocol.** The HMAC key is `K = SHA-256(code)`. Each MAC is HMAC-SHA256 (the RustCrypto `hmac` crate,
compared in constant time) over `cx-pair-v1|<label>|` and then each field with a 4-byte length prefix.

1. **The joiner checks itself.** It checks the code's check symbol, its own name, host name, account and machine
   id (whatever the viewer would refuse, so a bad request never uses up an invite). Then it makes or reads its
   cx peer SSH key (peer_ssh.rs).
2. **The joiner asks.** It connects to `host[:port]` and sends its client key, its identity (name, host,
   account, machine id, SSH host key), the viewer address it actually dialed, a 16-byte random nonce, and
   `HMAC(K, "join", those fields)`.
3. **The viewer checks.** If 3 requests failed within the last minute, it refuses without checking. A wrong MAC
   (or an unreadable request) is refused and counted, and the invite stays open; the 20th failure cancels the
   invite. A right MAC **uses up the invite before anything else**. Then it checks the dialed address against the
   address the connection really reached (a mismatch means a relay), and checks the identity. An id that is
   already enrolled is refused (see the threat model). Any failure after the invite is used up cancels it, and
   the listener closes.
4. **The viewer pairs.** It installs the joiner's key as `no-agent-forwarding,no-port-forwarding,
   no-X11-forwarding,no-user-rc … cx-managed-peer` (the existing peer_ssh line writer). It pins the joiner's host
   key and enrolls the joiner as a **core** device at `account@<connection source>`, with the claimed
   `machine-id:account` as its id.
5. **The viewer answers** with its SSH host key (`/etc/ssh/ssh_host_ed25519_key.pub`, or `--host-key FILE`), its
   cx client key, its id, name, host and account, and `HMAC(K, "accept", the request nonce and those fields)`.
   Then the listener closes.
6. **The joiner verifies** the accept MAC. Only then does it pin the viewer's host key, authorize its client key
   the same way, and enroll it as core at `account@<the host it dialed>`. If the viewer's id is already enrolled
   here, the joiner refuses too. The viewer has then recorded the joiner already, and the error says to revoke it
   there before joining again. `cx device sync` then verifies both directions with the existing pairing.

cx device targets carry no port, so the pairing assumes sshd on port 22 on both sides; the invite port is only
for the pairing exchange. `cx mesh revoke NAME --backend ssh-overlay --yes` removes the device, its pins and
exactly its `cx-managed-peer` line. `cx device remove NAME` does the same (its JSON reports `ssh.unpinned` and
`ssh.authorizations_removed`). `cx mesh teardown --backend ssh-overlay` has nothing to cancel: an invite exists
only while its `cx mesh invite` runs. Invites, joins and closed invites are in
`~/.local/state/cx/mesh/ssh-overlay/audit.jsonl`.

**Threat model.**

- **What the code is.** A short bearer secret, good for one join before it expires. Deliver it over a channel you
  trust for integrity and confidentiality: a terminal you see, a password manager, an end-to-end encrypted
  message. **Whoever sees the code before it is used can join first**, as a *core* device: their key gets a full
  interactive shell as the viewer's account (only forwarding, the agent and user rc are disabled). Treat the code
  like a password for that account. Mitigations: single use, a listener that runs only while you wait for the
  device, `--name` to spot surprises, the audit log with the source address, and `cx mesh members` and revoke to
  check and undo.
- **Online guessing** is bounded by single use and the rate limit: at most 3 checked guesses a minute and 20 in
  all before the invite is cancelled, against 2^50 codes.
- **A man in the middle on the first TCP exchange** (someone on the mesh or LAN path) can read the joiner's
  request. That gives them one HMAC, from which they could brute-force the 50-bit code offline. They cannot
  complete the join meanwhile: changing any field of the request breaks its MAC, and relaying it as is installs
  the joiner's own key, not theirs. They cannot forge the accept without the code, so the joiner pins nothing
  from them. A joiner that sees a failure says so; **cancel the invite** (Ctrl-C on the viewer) and make a new
  one. On the viewer, any failed handshake with the right code cancels the invite itself. Online, the rate limit
  above applies to them like anyone else.
- **Network exposure.** The listener only binds mesh/LAN addresses and only while the invite is open. Each
  connection is bounded in size and time and served one at a time, so a flood can delay a join but not exhaust the
  host; it can also burn the invite (20 failures), which costs only a new invite.
- **Substitution.** An attacker who swaps the code *and* the host the joiner is told can make the joiner pair with
  their host. Only the delivery channel stops that.
- **What the joiner's key may do.** Everything the viewer's account may do, in a shell: `cx-managed-peer` keys
  have the options of automatic core pairing, with no forced command. Agent forwarding is never used.
- **Identity claims.** The joiner names its own `machine-id:account`, and nothing proves it. An invite holder
  could claim the id of a device that is already enrolled, for example a directed device or an alias with no SSH
  pins. Overwriting that record would move its target to the attacker's address and promote it to core. So an
  invite only ever adds a **new** device: an enrolled id is refused, and the existing record, its target and its
  access are left alone. The refusal happens before any key is pinned or authorized, and the invite is used up.
  Re-pairing a known device means removing it first (`cx mesh revoke NAME --backend ssh-overlay --yes` or `cx
  device remove`). A new endpoint already enrolled for another device is refused as well. The same new-only rule
  applies on the joiner side.
- **Out of scope.** A compromised viewer or joiner, and a viewer whose sshd host key was stolen.

## Tests

`python3 tests/mesh_ssh_overlay.py target/debug/cx` pairs two temporary cx identities on loopback (debug builds
read the fixture's `CX_JOIN_MACHINE_ID`), each `cx mesh invite` listening on `127.0.0.1` on a free port:

- an invite holder claiming an already-enrolled device id: refused, that record untouched, the invite cancelled
- a wrong code refused with the invite left open, and Ctrl-C closing another invite
- the join, the pins and keys it leaves on both sides, and no copy of the code on disk
- the key the viewer installed logging in to a disposable loopback sshd that serves the viewer's
  `authorized_keys`
- a replay (nothing listens any more), a mistyped code refused offline, the rate limit refusing even the right
  code after 3 wrong ones, and expiry closing the listener
- after every invite, the listener port closed
- teardown, revoke and `cx device remove`
- `cx mesh backends` with a fake `tailscale` on PATH, and no WARP claim without a WARP config
- the real `~/.ssh/authorized_keys`, `~/.ssh/known_hosts` and cx state compared by content hash before and after

The Rust unit tests cover the code (encoding, decoding, the check symbol catching every typo and neighbour swap),
the HMACs (against RFC 4231), the listen addresses, the viewer's checks (wrong code, tampered key, changed
address, expiry, single use, a relay, the rate limit and the cancelling cap), forged and replayed answers on the
joiner, and the listener over real sockets (an oversized frame, a silent connection cut off, expiry, the port
closed), plus ranges, collisions and backend mapping.

`python3 tests/mesh_util.py` covers the profile excludes, the config, setup against a fake Cloudflare API
(idempotent, dry run read-only, teardown complete and its dry run), a dry-run join, and the onboarding
helpers (`token-url`, `verify`, `zones`) against fakes. `node tests/mesh_worker.mjs` runs the Worker
against an in-memory D1 and a fake API, through invite, enrol, register, leave, revoke, and the rate limits.
Neither touches an account.
