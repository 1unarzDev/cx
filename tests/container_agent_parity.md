# Container agent parity acceptance

Base: `970f9f8`. Tests are isolated from production implementation. The test
author executes pure self-tests only; root serializes all executable integration
and owned-container runs.

## Commands

```sh
python3 -B tests/container_agent_parity.py --self-test
# Public helper/CLI with fake Docker only; no daemon, SSH or tmux server:
python3 -B tests/container_agent_parity.py /absolute/new/cx --fixtures
# Optional independently built old helper compatibility check:
python3 -B tests/container_agent_parity.py /absolute/new/cx --fixtures --legacy-binary /absolute/old/cx
# ROOT ONLY: disposable real Docker/tmux; image must already exist locally:
python3 -B tests/container_agent_parity.py /absolute/new/cx --run-owned-container --image ubuntu:24.04
```

The last command requires local Docker access, tmux, a static C compiler, and an
installed native-architecture image containing `/bin/sh`, Bash, `sleep`, `mkdir`,
`chmod` and `test`. It cannot pull or build an image. A unique owned container is
started with `--network=none`, a temporary proof-directory mount, and synthetic
ELF executables named Codex/Claude. They record only their known fixture argv/cwd
and wait; they do no inference and read no provider config. The host environment
is allowlisted and default tmux discovery uses private `TMUX_TMPDIR`. The managed
server is prestarted at its exact temporary socket, bypassing service startup.
Cleanup removes only the returned owned container ID and owned tmux sockets.
Use the same container engine/root permissions as normal CX; this fixture does
not change Docker contexts, SSH trust, host networking, or provider credentials.

## Protocol contract

- `container_providers {scope}` returns `{providers:[...], scope:<exact input>, observed_at:<seconds>}`.
- `container_agent_create {scope, request, yolo}` is the versioned agent launch;
  `request` is CreateSession with a required intentional `resume` boolean in tests.
- Supported agent profiles are Codex and Claude; shell launch stays on the existing path.
- `request.directory` must equal `scope.folder`.
- Resume selects native history: `codex resume` or `claude --resume`.
  The fixture rejects `--last`/`--continue` by exact argv comparison.
- Existing YOLO flags append without changing the explicit history choice.
- The old `container_create` operation must refuse `resume:true`, never quietly
  launch fresh work. Old binaries should reject the new unknown operation.
- Agent UI/CLI uses `container-agents-v1` capability evidence before sending the
  new operation; absent support cannot justify legacy fallback for a history request.

## Pairwise executable matrix

| Layer | Cases | Required evidence |
|---|---|---|
| Fake Docker public helper | Old engine/start/user, replaced full ID, stopped container × provider discovery and history creation | Actual inspect occurs; error returned before `docker exec`. Unknown-protocol rejection does not count as passing a scope check. |
| Fake Docker public helper | Requested directory differs from scope folder | Refusal; request cannot silently override execution location. |
| Fake Docker public helper | Same path on two IDs with different providers; engine/user/start changes after executable probe | Evidence remains specific to container/user/folder; post-probe change rejects instead of publishing old capabilities. |
| Real owned container | Installed Codex+Claude; Claude made nonexecutable | Exact installed subset from that container; exact scope/time; no `/tmp/cx-tools-0` created by discovery. |
| Real launch chain | Codex/Claude × fresh/native-history × Default/YOLO | All 8 actual argv vectors plus quoted/Unicode cwd survive host session → container-run → container-shell → provider. |
| Public CLI flags | `container-providers --directory`; `container-new --provider codex --resume --yolo --directory` | CLI preserves exact scope and explicit native picker/permission arguments, in addition to direct framed-operation coverage. |
| Idempotency | Repeat each exact creation key/request; change provider/resume/YOLO under one existing key | Same session ID on exact repeat; changed intent rejected; exactly one live runtime. |
| Existing runtime | Watch one already-running history picker session | Same terminal, one session, no replacement agent argv, termios restored. |
| Compatibility | New binary receives resume on old create op; agent create receives shell | Explicit refusal; no managed session created. |
| Container lifecycle | Actual restart after observations | Old scope cannot advertise providers or launch; no terminal created. |

This is a bounded pairwise suite, not exhaustive coverage of every shell, image,
Dev Containers CLI, provider version or container engine.

## Required root UI and further adversarial checks

The following are not established by this runtime harness:

1. **Scope of provider evidence:** host has both agents; A has Codex only; B has
   Claude only; all three share `/workspace`. Switch A→B while A's capability reply
   is delayed. B must never display A/host choices. Include user or `started_at`
   change with same ID, plus stale scope returned in the response.
2. **Discovery race:** runtime fake checks cover lifecycle/user/engine changing
   during the probe. UI must also reject a valid but delayed response whose scope
   differs from its current menu; that viewer-level race remains separate.
3. **Unsupported helper:** missing `container-agents-v1`, unknown new operation,
   and helper update failure must retain safe shell/files navigation while refusing
   agent history launch. Never turn a history action into fresh work.
4. **Exact runtime picker:** two sessions in the same folder and two containers
   with the same folder/provider must remain distinct. Enter/Watch only the chosen
   runtime. New fresh/history actions stay explicit and never fire on picker refresh.
5. **Capability freshness:** failed provider refresh should not leave stale choices
   actionable indefinitely. No host-provider fallback on a missing container result.
6. **Older launch records:** new records bind provider/resume/permission and are
   exercised live. Existing records without new `.launch` metadata must refuse
   resumed retry rather than guess historical launch semantics; root should retain
   the runtime unit tests for this migration boundary.
7. **Inner helper compatibility:** the provider capability belongs to the host
   helper, but the final decoder runs in a copied container helper. A release must
   invalidate previously installed helpers by version or negotiate support. Reusing
   an old binary under the same version string can otherwise lose/reject the new
   resume tail. Verify that failure never invokes a fresh provider command.
8. **Dev Containers CLI execution:** real CLI config/remoteUser/remoteEnv path is
   separate from Docker fallback tested here. Extend an owned fixture after the
   installed CLI is verified; don't run existing `devcontainer_cli_live.py` unchanged
   because it can start image/workspace operations beyond this bounded test.

Existing `container_agents_live.py` has useful raw input/clipboard-style coverage,
but inherits caller default tmux/provider environment and permits implicit image
pull. Its physical acceptance claims do not substitute for this isolated matrix.
Actual provider auth, history picker interaction, terminal completion/scrollback,
and hardware/container environment acceptance remain separate root/user checks.

## Recorded author execution

Five pure self-tests passed: argv expectations exclude latest shortcuts, quoted
folder request preservation, fake Docker script compilation, and exact scope
evidence validation, plus in-process fake probe/lifecycle-change behavior.
`git diff --check` passed. No Docker, tmux, service, provider,
network, PTY, or public-helper integration command was run by the test author.
