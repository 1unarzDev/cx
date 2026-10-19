# CX status

## Terminal startup — v0.1.71

New terminals initialize detached while CX animates a loader, then hand off at
first visible content. Silent programs remain attachable after five seconds;
startup exits report an error in the panel. Remote readiness needs updated helpers.
Installed binaries and active sessions were left untouched.

## Session reliability — v0.1.69

Implemented startup-folder navigation, background cache reconciliation, dead-shell
filtering, exact container lifecycle checks and rediscovery of existing tmux-backed
Docker exec terminals. Native agent names/directories/tasks refresh with process
identity checks; new supported Codex launches use separate processes. Sessions show
observed activity and offer task details with `s`. Claude question descriptions
receive a solid default-cell background while retaining their existing colors.
Device removal/address changes invalidate old caches and in-flight observations.

## Remaining integration and acceptance

- Blastoise and Squirtle were unreachable during bounded SSH checks. Physical
  reconnect/attach acceptance needs those devices online and updated helpers.
- Silent model/network waits and silent inner-container commands have unknown
  activity. Unsupported or ambiguous native metadata remains unavailable.
- No installed binary replacement, provider restart, tmux reset or fleet deployment
  was performed; multiple active local Codex sessions must remain running.

Actual test evidence is in [VALIDATION.md](VALIDATION.md). Design findings and
limits are in [the audit](research/session-reliability-audit.md). Previous status,
including unrelated hardware/network acceptance, is preserved in
[the archive](docs/archive/status-2026-10-19.md).
