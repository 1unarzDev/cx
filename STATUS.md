# CX status

## Automatic core mesh — v0.1.76

Enrolling/promoting a core device, opening CX, or a background retry reconciles
reciprocal SSH access with every other core device. Directed devices are excluded.
Each host retains its own dedicated private key. Managed host pins stay in CX state;
personal SSH config/known_hosts are unchanged. New directories use mode 700 even
under umask 0002; safe empty legacy roots can recover without rotating keys.

LIVE: Innovation, Creation and Tranquility each completed real session-list requests
to both other hosts: all six directions passed. Ordinary startup formed the mesh
after Tranquility woke up; repeated sync reports all three pairs ready with no
pending edges. Tranquility's legacy Innovation entry was migrated from Directed to
Core, and Creation's empty v0.1.75 managed directories were repaired. All original
session identities and personal SSH configuration hashes are preserved.
Signed v0.1.76 is installed on all three, and all six session-list requests
passed again after deployment. Release validation passed on retry. No provider/tmux restart or live networking change occurred.

## Session adoption (Unreleased, `feat/session-adoption`)

Outside-CX terminals are listed and can be moved into managed tmux through the guarded cx-reptyr relay. Local fixtures pass: synthetic busy multithreaded agents kept their identities, saw no hangup or EIO, and output paused at most about 14 ms over repeated randomized moves. Target exits, relay kills and forged identities were injected and left nothing behind. Pending:
- Real Claude/Codex mid-turn on a real terminal and over SSH.
- The privileged `cx device ptrace enable` on a real host.
- A signed cx-reptyr release asset.
- Fallback resume for hosts without ptrace.
- In-container (shim-held) terminals.

## Session behavior already released

Startup-folder navigation, background cache reconciliation, dead-shell filtering,
container lifecycle checks and rediscovery of tmux-backed Docker exec terminals are
implemented. Native agent names/directories/tasks refresh with identity checks;
supported Codex launches use separate processes. Sessions show observed activity and
offer task details with `s`. Terminal startup shows an animated loader. Codex falls
back to a neutral provider label; Claude preserves terminal transparency.

## Remaining integration and acceptance

- Blastoise and Squirtle remain Directed and were unreachable during earlier bounded
  checks. Physical reconnect/attach acceptance needs them online and updated helpers.
- Silent model/network waits and silent inner-container commands have unknown
  activity; unsupported or ambiguous native metadata remains unavailable.

Removal/demotion prevents future enrollment but does not revoke existing SSH keys.
Automatic setup requires supported helpers and direct port-22 connectivity; offline
peers remain pending and retry. Changed identity/key pins require verification.
Work was integrated through an isolated worktree; the original conflicted checkout
was left untouched. Test evidence is in [VALIDATION.md](VALIDATION.md); prior design
findings are in [the audit](research/session-reliability-audit.md).
