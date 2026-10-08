# Robot access and preserved-LAN internet sharing

Implementation branch: `feature/robot-mesh-sharing`. This is a tested access policy and sharing preview, not a completed live sharing service.

## Access through the mesh

A mesh viewer opens SSH to every hop using its own identity. OpenSSH ProxyJump supplies transport; no private keys or forwarded agents are installed on robots. The robot account accepts explicitly enrolled mesh public keys. Discovery describes a possible path, while authorization permits the original viewer to authenticate to its destination and every transit host. These are separate facts.

`cx add roboboat@ADDRESS --via tranquility --minimal` installs a signed architecture-matched helper without installing optional packages. Existing full enrollment still checks the normal utilities. Files and read-only network observation work on a minimal host; terminal sessions need tmux and PDF conversion needs the usual converters. Enrollment does not make those tools available.

The directed graph has mesh/downstream roles, approved reachability edges, independent viewer grants and authenticated timestamps. `cx access FROM TO --graph FILE` reviews the shortest approved route, bounded to four jumps. `--install-route` stores that path for an already enrolled destination. The input is an explicit administrator policy snapshot, not independently authenticated evidence from another host. Every transit host needs a grant for the original viewer. Duplicate targets are rejected: use separate aliases for equal private addresses behind different gateways. A downstream node cannot acquire a mesh SSH grant. Fresh successful checks in both directions establish a bidirectional label; an absent reverse grant means **not granted**, not firewall-enforced denial.

Future requests discard helper channels when the stored path changes. Separate native SSH sessions remain attached. Routes are viewer-local decisions; these changes do not yet distribute graphs or revocations automatically to other mesh hosts. For now each viewer needs enrollment, destination trust, its own approved route and its own authentication check. A newly discovered endpoint does not silently install trust or credentials. Further downstream nodes can be represented and routed with the same graph, including a robot as a transit host.

## Gateway selection without changing robot profiles

The intended sharing gateway is tranquility's existing `192.168.0.2`. DD-WRT at `.1` owns its address; CX must never adopt `.1` as an alias. Fresh tranquility DHCP observation advertised routers `.1`, despite the intended `.2` setting. This discrepancy requires measurement, not guessing or a router reset.

Choose the smallest effective change:

1. Inspect host addresses, robot defaults/local routes/DNS, fresh DHCP option 3, gateway ownership, and the actual forwarding policy route. A host's own public route does not necessarily describe forwarded traffic, particularly with WARP.
2. Retain a robot's current gateway when it already names the host, or when fresh robot egress plus host NAT-counter evidence proves the existing path exits through that host. A ping or the existence of a default route alone is insufficient proof of sharing.
3. Otherwise trial a scoped, temporary robot default through the host's existing LAN address. Preserve the old default, interface addresses, local routes and connection profile. Determine metrics and rollback from the actual route snapshot; do not blindly replace a default. Arm independent host-local rollback before either side changes.
4. Only consider a new host alias if no existing address can serve the robot and a fresh ownership/conflict check establishes that the candidate is unused. Do not assume silence proves ownership. Address adoption needs a separate reviewed transaction.
5. Leave DD-WRT DHCP configuration alone. An administrator may separately choose option 3 `.2`; that affects other clients and is unnecessary for a per-robot trial.

`cx share-plan ID --observation FILE` accepts bounded fresh evidence, produces dedicated nftables tables, gateway actions and DNS prerequisites, and **never applies changes**. A working indirect gateway is retained only with fresh end-to-end evidence. Otherwise the plan proposes a trial route that explicitly preserves the original gateway. Plans always report `ready_for_apply: false` pending the production transaction implementation and enforcement validation.

## Internet and reverse access acceptance

The production transaction must use a root-owned narrowly scoped helper, an owned lock, durable rollback ledger and a host-local watchdog armed before application. It must refuse existing table-name collisions and remove only its owned resources. It must restore exact temporary routes/DNS changes on timeout, disconnect or failure; reboot recovery and persistence need explicit ownership too. Do not run a user-writable executable as a privileged persistent service.

For an established robot LAN, dedicated source-scoped NAT is preferable to replacing the Ethernet profile with NetworkManager shared mode. Preserve unrelated firewall tables and normal robotics traffic. Existing firewall chains can still reject traffic after an accept in CX's chain; real packets must verify the complete path.

Reverse SSH enforcement is a separate acceptance gate. The preview blocks IPv4 new TCP/22 traffic from listed robot addresses to the sharing host and listed mesh addresses while preserving established replies. This does not cover direct robot-to-mesh LAN paths that bypass tranquility, alternate SSH ports, IPv6, address changes, or arbitrary relays. Inventory every mesh LAN/overlay SSH listener and robot IPv4/IPv6 path; place owned enforcement on the actual paths before claiming robots cannot SSH back. Absence of reverse public keys is insufficient. Scoped ordinary-network protection is not isolation from an administrator who can alter the robot's firewall or identity.

After trial activation, verify each robot's DNS, public HTTPS, observed source-NAT counters, preserved management/robot-local connectivity, permitted mesh-to-robot SSH, and negative reverse checks on every inventoried path. Commit only after these succeed. Missing privilege/evidence must leave sharing unconfirmed rather than advertise internet access.

## Observed live state

- Blastoise is Fedora CoreOS ARM64, with `.147` and `.152` on the same device. Its existing host key was verified through tranquility. CX-tagged inbound public keys for innovation, tranquility and verybeautifulserver were installed while preserving unrelated entries. No reverse keys were installed.
- Innovation authenticated through tranquility to blastoise, installed the signed v0.1.31 ARM64 helper in minimal mode and enrolled `roboboat@192.168.0.147`. Its network helper responds. Its existing default still points at `.1`; robot addresses/profiles/routes were not changed. Earlier DNS/public-internet checks failed. No subsequent live internet success is claimed.
- `.153` is squirtle-jetson, confirmed by its own trusted host key and hostname. It has not been enrolled as squirtle-odroid. Squirtle-odroid's address remains unresolved.
- Tranquility already has `.2`, IPv4 forwarding and WARP; noninteractive sudo is unavailable. No router, WARP, firewall, profile or robot networking changes were made during this implementation.

## Verification

The final locked Rust suite passed 451 tests with one protected live test ignored. Ordinary-account enrollment fixtures verify preservation/idempotence, unsafe-key-file refusal, minimal installation without optional utilities and full-mode prerequisite refusal. Native OpenSSH jump fixtures verify background access never prompts and conflicting existing routes are preserved.

Namespace lifecycle tests compile and apply real nftables rules while preserving addresses, routes and unrelated tables. Separate three-node packet tests verify source NAT, IPv4 reverse TCP/22 denial, permitted mesh initiation/replies and rollback to baseline. These isolated tests do not establish live robot DNS/public HTTPS, IPv6 protection, destination trust distribution or privileged recovery/persistence.
