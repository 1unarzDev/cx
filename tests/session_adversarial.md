# Adversarial session acceptance

This is a practical pairwise matrix, not a claim to cover every shell, tmux,
terminal emulator, container runtime or operating system configuration.
The harness exercises the public CLI/helper rather than importing session logic.

## Execution and isolation

```sh
python3 tests/session_adversarial.py --self-test
python3 tests/session_adversarial.py /absolute/path/to/cx --fixtures
```

The six self-tests spawn no programs. `--fixtures` uses fake Docker and SSH
executables, temporary state, and an allowlisted environment. Container checks
prove that stale engine/start/user/ID and stopped-container evidence cannot
reach `docker exec`. The offline check proves an actual transport attempt fails
without returning a misleading empty session list. Neither contacts a daemon
or network. The fixture `systemd-run` always refuses execution.

Root alone runs the live mode, serialized with other host mutations:

```sh
python3 tests/session_adversarial.py /absolute/path/to/cx --run-owned-tmux
python3 tests/session_adversarial.py /absolute/path/to/cx --run-owned-tmux --case aliases
python3 tests/session_adversarial.py /absolute/path/to/cx --run-owned-tmux --case benchmark --samples 5
```

Live mode starts disposable servers at exact sockets under one private temporary
directory. It prestarts the managed server before public `cx new`, preventing
CX's user-service startup path. Default discovery is redirected with private
`TMUX_TMPDIR`; no personal server is enumerated. Only the owned default/managed
sockets are stopped. Synthetic agents are copies of Bash named `codex` and
`claude`; no actual agent, credentials, provider configuration or inference is
used. Shell rc files are bypassed. Cleanup runs after each case, including red
assertions. No terminal capture/transcript is collected or printed.

## Executable cases

| Case | Pairwise dimensions | Required behavior |
|---|---|---|
| `aliases` | Managed socket and symlinked default socket × two same-name/same-folder sessions × inherited managed/default `$TMUX` | Exactly two runtime rows, no alias duplicate, no legitimate-session collapse, same inventory inside/outside tmux. |
| `foreground` | Shell → synthetic Codex → shell → Claude → shell × identical custom labels/folders × quotes/tab/newline/bidi label | Two terminal runtimes remain distinct; provider follows foreground; original terminal identity and exact custom label survive transitions. UI sanitization is a separate acceptance check. |
| `dead-agent` | Public managed agent creation × `remain-on-exit` × synthetic agent exit | Retained dead standalone pane is not advertised as live attachable work. Its tmux pane is independently verified dead before asserting. |
| `dead-active-pane` | External session × dead selected pane × another live pane | Session remains discoverable; single-pane dead filtering must not erase multi-pane work. |
| `distinct-servers` | Two independent servers × same name/folder | Keep both runtimes; deduplication must use observed runtime/server identity, never just label/path. |
| `stale-stop` | Managed live shell × wrong PID/start/boot | Reject each stale stop, retain the exact original live runtime. |
| `benchmark` | 1/10/30 managed same-folder sessions × repeated CLI discovery | Correct row count; emit first sample, p50/p95 milliseconds and tmux subprocess count. No speed threshold is asserted without a baseline. |
| fixture `container-scope` | Old engine/start/user/ID or stopped state × file request | Reject before any container execution; replacement never silently retargets old scope. |
| fixture `offline` | Enrolled remote identity × simulated DNS failure | Bounded transport attempt, nonzero exit, no fake empty/live list. |

## Root UI and future endpoint acceptance

These are **not covered** by the CLI harness. Root owns the UI/cache/pin tests;
the runtime fixer owns session inventory logic.

| Scenario | Exact minimal test | Expected result |
|---|---|---|
| Alias discovery with a selected/pinned row | Select a session; introduce an alternate route to the same observed runtime; refresh. | One row and one pin; selection stays on that runtime; ownership does not flip. |
| Valid duplicate work | Open two sessions in the same project with the same label; pin one; change ordering by rename. | Both remain visible; selection/pin follows the chosen runtime, not row index/name. |
| Cached offline work | Cache two remote sessions, then make the fake transport fail; invoke Attach. | Last-seen state is marked stale; no false live/empty state; no new session as an automatic fallback. |
| Vanished selected row | Select session A; it exits; B sorts into the same position; press Enter. | No unintended attach to B without selection reconciliation and visible feedback. |
| Session chooser refresh race | Open chooser with one existing session; create another externally while refresh is pending. | Existing choices remain stable; New remains explicit; refresh never automatically creates a third session. |
| Container restart | Cache scoped session; change `started_at`; attempt attach/resume. | Refuse stale runtime; distinguish new terminal/native resume from attaching the old process. |
| Container replacement | Same friendly name and workspace but different full container ID. | No automatic retargeting; unique verified workspace evidence and explicit replacement action required. |
| Two external sockets both contain `$0` | Register two owned external endpoints, each first session `$0`; select one. | Server/socket identity disambiguates Attach and pins. Current public API cannot register arbitrary sockets; this remains a contract/integration gap, not a passing harness case. |
| Existing container tmux | Container created outside CX with preexisting tmux session; open container session chooser. | Select and attach same runtime without changing server options or creating a new shell. Current host-only attachment contract needs extension. |
| Nested actual attachment | Launch CX inside owned tmux; select same-server, different-server, remote session. | Defined switch/return behavior without accidental nested layers, key interception or modifying another client. Current `attach()` rejects inherited `$TMUX`; discovery-only nested checks do not establish attachment support. |
| Terminal close vs process exit | Attach, close viewer, reopen; separately exit agent then reopen. | First reattaches same process; second shows ended/recovery evidence, not a live ghost. |

Physical acceptance remains separate: shell completion, Ctrl+C/Z/D, Claude/Codex
editor takeover, bracketed paste, scrollback, mouse selection, clipboard, resize,
and detach. Do not infer physical behavior from fixture metadata results.

## Timing interpretation

Run the identical harness against before/after binaries on the same machine,
with no concurrent fleet/host test activity. Record binary SHA256, build profile,
tmux version and sample count separately. `first_ms` is the first measured fresh
CLI process after session setup; it is **not** a cold-machine measurement.
All samples include CLI startup and metadata discovery. The tiny counting wrapper
adds fixed per-tmux-invocation overhead to both builds; subprocess counts provide
independent evidence for batching. Five samples are a quick baseline, not a
statistically strong tail estimate; use `--samples 30` for a review run. Neither
fixture-mode elapsed times nor synthetic Bash timings establish real fleet,
container, or provider-title performance.

## Recorded execution by the test author

At base `8f369e6`, the author ran the six harness self-tests and fake Docker/SSH
CLI cases only. Both fixture cases passed against
`/home/lunarz/cx/target/debug/cx`, SHA256
`c5e3a987570f9fc7a11d41d90c6dd542f9dc6727d41dfce171a98785347ea948`.
That binary is externally built; this records its identity, not an assertion
that it was compiled from the harness base. No live tmux, Docker, service,
physical terminal, benchmark or actual-network test was run by the author.
Root records live outcomes separately after integration.

## Checked native attachment PTY supplement

```sh
python3 -B tests/session_adversarial_attach.py --self-test
# Root only, after checked-attach CLI integration:
python3 -B tests/session_adversarial_attach.py /absolute/path/to/cx --run-owned-tmux
```

This supplement requires `native-attach-checked PAYLOAD` and
`native-attach-check PAYLOAD SERVER_PID SESSION_ID CREATED`. PAYLOAD is standard
base64 of JSON `{"session": <exact returned Session>, "observe": false}`.
It validates the same-queue predicate, actual managed/external client attachment,
read-only watch input rejection, termios restoration, external and managed global
option/inventory preservation, changed server generation, wrong creation/numeric
identity, renamed original with a new same-name peer, and dead selected runtime.
Two owned servers are queried by exact socket; PTY bytes are discarded rather
than captured. The original six self-tests and the payload self-test ran; live
checked attachment awaits root execution against the integrated binary.

The tiny interval between predicate acceptance and tmux attachment is not forced
by this supplement. Root's same-command-queue implementation plus its targeted
predicate tests must establish that ordering; a preflight-only pass cannot.
