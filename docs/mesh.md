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
interface list only: it never runs `tailscale status` or takes the ssh-overlay invite lock. `cx mesh backends` is
where each backend's full status is read.

## ssh-overlay: `cx mesh pair`

```sh
# the viewer (a device already in your cx mesh)
cx mesh invite --backend ssh-overlay [--hours 1] [--name rover] [--address IP]... [--short]
# the new device (cx installed; Linux)
cx mesh pair 06RRQ-B741Q-E4FQ7-...   # or, with --short on a Cloudflare mesh: cx mesh pair mesh.example.org/7KQ2-M9XD-4HT0
```

`cx mesh join` is the WARP join above; `cx mesh pair` is this one.

A **direct code** is Crockford base32. Input ignores case, `I`/`L` read as `1` and `O` as `0`, and dashes are
ignored. It holds:

- a version
- an 8-byte one-time token
- the first 10 bytes of the SHA-256 of the viewer's SSH Ed25519 host key
- the expiry (to the minute)
- the viewer's SSH account
- 1 to 4 addresses (IPv4/IPv6; the format has room for a port, but cx pairs over port 22 only)
- a CRC-16 checksum

With one IPv4 address it is about 60 characters. A single mistyped character always fails the checksum.

A 10 to 12 character code cannot carry an address and a host key pin at a useful strength. So the **short code**
(12 symbols: 50 random bits and a CRC-10 check) exists only with a `cloudflare-warp` mesh Worker. cx seals the
direct code (SHA-256 keystream with HMAC-SHA256, keys derived from the short code) and stashes it with
`POST /v1/admin/pairs`. The Worker stores only the seal and a separate hash of the short code. `cx mesh pair
<host>/<code>` takes it once (`POST /v1/pair/redeem`, rate limited per address) and unseals it.

**The protocol.**

1. **The viewer creates the invite.** The token derives an Ed25519 *invite key* (seed = SHA-512 of a label and
   the token). The viewer appends only its public half to `~/.ssh/authorized_keys`:
   `restrict,command="<cx> mesh pair --redeem <id>",expiry-time="<UTC>Z" ssh-ed25519 … cx-join-invite:<id>`.
2. **The joiner checks the host key.** It decodes the code and checks the expiry, the port, and its own name,
   host name, account and machine id (what the viewer would refuse, before any invite is used). It runs `ssh-keyscan -t ed25519`
   on each address in turn and accepts a host key only when its digest matches the code. A mismatch stops the
   attempt before any authentication.
3. **The joiner connects.** It writes the accepted key as the only known host and connects with
   `ssh -F none -o StrictHostKeyChecking=yes -o IdentitiesOnly=yes`, with no agent and no forwarding, using the
   invite key derived from the code.
4. **The viewer redeems.** sshd runs only the forced redeem command. The command takes one framed request (`CX1
   <len>` plus JSON, at most 64 KiB):
   - It burns the invite first (single use) and removes its `authorized_keys` line.
   - It installs the joiner's own Ed25519 key as `no-agent-forwarding,no-port-forwarding,no-X11-forwarding,no-user-rc
     … cx-managed-peer`.
   - It pins the joiner's host key and enrolls the joiner as a **core** device, at `account@<SSH_CONNECTION
     source>`. The device id is the joiner's claimed `machine-id:account`. An id that is already enrolled is
     refused before any key is installed (see the threat model).
   - It answers with the viewer's identity and its cx client key.
5. **The joiner pairs back.** It pins the code-verified host key and authorizes the viewer's client key the same
   way, then enrolls the viewer as core. If the viewer's id is already enrolled here, the joiner refuses too.
   The viewer has then recorded the joiner already, and the error says to revoke it there before joining again. `cx device sync` then verifies both directions with the existing pairing.

`cx mesh revoke NAME --backend ssh-overlay --yes` removes the device, its pins and exactly its `cx-managed-peer`
line. `cx device remove` does not yet remove the pins and that line, so use revoke for a paired device.
`cx mesh teardown --backend ssh-overlay --yes` cancels pending invites, and every invite line no longer pending
(expired ones included) is removed then and whenever an invite is created. Invites, redemptions and refusals are
in `~/.local/state/cx/mesh/ssh-overlay/audit.jsonl`. cx device targets carry no port, so invites and joins refuse
any SSH port other than 22.

**Threat model.**

- **What the code is.** It is a bearer secret and a pin. Deliver it over a channel you trust for integrity and
  confidentiality: a terminal you see, a password manager, an end-to-end encrypted message.
- **Interception.** Whoever holds the code first can join once, before it expires, as a *core* device: their key
  gets a full interactive shell as the viewer's account (only forwarding, the agent and user rc are disabled).
  Treat the code like a password for that account. Mitigations:
  - Codes are single use, and the default lifetime is one hour (at most 7 days).
  - Redeem is logged with the source address. `cx mesh members --backend ssh-overlay` shows the result, and revoke
    undoes it.
  - Bind the name with `--name` to spot surprises.
- **Substitution.** An attacker who swaps the code can make the joiner pair with *their* host. The checksum does
  not stop this: only the delivery channel does.
- **Network attacker (no code).** They cannot impersonate the viewer: the 80-bit host key pin needs a second
  preimage of SHA-256. They cannot use the invite: authenticating needs the token, at 64 bits, online only and
  rate limited by sshd, with expiry enforced by both sshd (`expiry-time`) and cx. Replaying the session is
  defeated by SSH itself.
- **What sshd allows the invite key.** `restrict` disables PTY, forwarding, agent and X11. The forced command
  ignores the requested command. The line disappears at redemption, at teardown, or when sshd's expiry passes.
- **What the joiner's own key may do.** Everything the viewer's account may do, in a shell: `cx-managed-peer` keys
  have the options of automatic core pairing (`no-agent-forwarding,no-port-forwarding,no-X11-forwarding,
  no-user-rc`), with no forced command. Agent forwarding is never used.
- **Short codes.** The Worker sees only a hash of the short code and the seal, and gives the seal out once. An
  operator of the Worker (you, the account owner) could brute-force the 50-bit code offline within its lifetime.
  The account owner is already trusted with the mesh. Rate limits and single use stop online guessing.
- **Identity claims.** The joiner names its own `machine-id:account`, and nothing proves it. An invite holder
  could claim the id of a device that is already enrolled, for example a directed device or an alias with no SSH
  pins. Overwriting that record would move its target to the attacker's address and promote it to core. So an
  invite only ever adds a **new** device: an enrolled id is refused, and the existing record, its target and its
  access are left alone. The refusal happens before any key is pinned or authorized, and the invite is still
  burnt. Re-pairing a known device means removing it first (`cx mesh revoke NAME --backend ssh-overlay --yes`
  or `cx device remove`). A new endpoint already enrolled for another device is refused as well. On the joiner
  side, the viewer's identity comes from a host whose key matched the code. The same new-only rule applies there.
- **Out of scope.** A compromised viewer or joiner, and a viewer whose sshd host key was stolen.

## Tests

`python3 tests/mesh_ssh_overlay.py target/debug/cx` runs `cx mesh pair` end to end against a disposable loopback
sshd, with two temporary cx identities (debug builds read the fixture's `CX_JOIN_MACHINE_ID`, which also lets its
sshd use a free port):

- an invite holder claiming an already-enrolled device id, refused with that record untouched
- the join, and the pins and keys it leaves on both sides
- a replay, refused
- a changed host key, refused before authentication
- a mistyped code, refused offline
- teardown and revoke
- `--short` end to end: cx, the embedded mesh.py and the Worker (`tests/mesh_worker_server.mjs`, the real
  worker.js over local HTTPS with an in-memory D1), then `cx mesh pair <host>/<code>` and its single use
- `cx mesh backends` with a fake `tailscale` on PATH, and no WARP claim without a WARP config

The Rust unit tests cover code encoding, decoding, the checksum and expiry, Ed25519 derivation (RFC 8032 vectors
and `ssh-keygen -Y` signatures), ranges and collisions, and backend mapping.

`python3 tests/mesh_util.py` covers the profile excludes, the config, setup against a fake Cloudflare API
(idempotent, dry run read-only, teardown complete and its dry run), a dry-run join, and the onboarding and
short-code helpers (`token-url`, `verify`, `zones`, `pair-stash`, `pair-redeem`) against fakes. `node tests/mesh_worker.mjs` runs the Worker
against an in-memory D1 and a fake API, through invite, enrol, register, leave, revoke, the rate limits and the
short-code stash and redeem.
Neither touches an account.
