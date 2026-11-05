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
`leave`, whose fetched script also runs with the variable removed), so the plaintext file can go. It needs these permissions:

- Account: Zero Trust Write, Access: Apps and Policies Write, Access: Service Tokens Write, Access: Organizations
  Read, Cloudflare One Connector: WARP Write, D1 Write, Workers Scripts Write, Account API Tokens Write.
- Zone, only with a domain: Workers Routes Write and Zone Read.

Setup gives the Worker its own narrower token (Zero Trust, Access and WARP connectors) as a secret binding. The owner
commands authenticate to the Worker with any active token of the account, which Cloudflare verifies.

Before the first setup, open one.dash.cloudflare.com once and pick the Free plan, so the account has a Zero Trust
organization.

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

## Tests

`python3 tests/mesh_util.py` covers the profile excludes, the config, setup against a fake Cloudflare API
(idempotent, dry run read-only, teardown complete) and a dry-run join. `node tests/mesh_worker.mjs` runs the Worker
against an in-memory D1 and a fake API, through invite, enrol, register, leave, revoke and the rate limits.
Neither touches an account.
