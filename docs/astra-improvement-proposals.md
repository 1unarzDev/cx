# CX session management and feature priorities

Make existing work easy to find and join across machines, regardless of whether CX launched it. Prioritize Claude, Codex, ordinary shells, Docker and devcontainers. A fast session switcher, accurate execution context and reliable attachment are the foundation for higher-level agent workflows.

The user identified agentic coding across machines as the leading workflow and clarified that weak external-session handling affects shells generally, not only containers. Three Astra specialists independently inspected workflow, performance and networking concerns. This proposal assesses committed source at `c60e1a2c0a5375717c1fd8e8bae7e31290ad0306`; the root checkout has unresolved merge conflicts and was preserved.

## Implemented milestone — 2026-10-23

The first slice now lives on `feat/session-workspace-20261023`. It adds a named runtime picker, all sessions with pins first, persistent ended-work bookmarks, exact-folder agent presence badges, bounded persistent navigation caches, obsolete-read coalescing, observation deduplication, dead-pane handling and generation-checked attachment. See [validation](../VALIDATION.md) for actual checks and remaining acceptance. The assessment below describes the original baseline; it is retained to explain the broader priorities, not to imply every proposed feature is implemented.

The user's settled preferences are **show all; pins first** and **keep ended pins as bookmarks; reopen deliberately**. No inactive-work hiding or automatic provider restart is enabled. The existing conflicted root checkout is preserved; integration uses the isolated branch.

## Next features, ranked by workflow value

The user selected **container agent launch and resume parity** as the next implementation cycle. The broader ranking below describes workflow value; implementation proceeds with that selected priority.

1. **Exact conversation continuation.** Show provider-native conversation ID, user-visible name and last observed time before resuming a stopped Claude/Codex thread. An attached shell, tmux runtime and provider conversation are separate identities. Avoid guessing from the folder or “latest”; do not merge distinct threads or kill duplicate-looking processes.
2. **External endpoint coverage and nested switching.** Register custom tmux endpoints and define same-server switch/return behavior before offering a persistent terminal launch profile. Inner-container tmux needs its own endpoint adapter and scope validation.
3. **Container agent parity.** Probe actual container/user capabilities; expose Claude/Codex and verified resume intent alongside existing shells. Preserve scope across stop/restart/rebuild and show missing evidence honestly.
4. **Git worktree context.** Add branch/worktree labels to session and folder context, with a deliberate “start agent in this worktree” action. Keep Git mutations explicit.
5. **Background build/test receipts.** Named command runs with actual exit status, completion time and selected artifacts make remote agent work reviewable after disconnection.
6. **Local service forwards.** A deliberate loopback SSH forward for a remote web app/notebook, using the saved route and showing its actual listener state.

## Dedicated QoL and visual ideation

The QoL specialist's priority order was: identifiable session choices; pins that promote the same row; folder presence badges; stable cached views through refresh/failure; one runtime per row with execution context; honest empty/filtered/unavailable states; and a shortcut back to the previous exact runtime. The first six informed this milestone. Returning to previous work remains a proposal.

Useful follow-on visual choices:

- A compact recent-location trail (device → container → worktree → folder) with one-key return. Preserve scope rather than relying on matching path strings.
- A quiet selected-session context card showing full name, origin, location and observation age, especially when table columns truncate names.
- Worktree/branch labels beside folder badges, with a textual fallback in narrow or monochrome terminals.
- Optional attention markers only from explicit provider events, such as “waiting for input” or a reported error. Process presence alone cannot establish thinking, idle, completion or task success.
- A named build/test result marker that opens its exit receipt and artifacts. Avoid animated activity decorations that imply unobserved progress.

The current folder badge is intentionally modest: `Claude 2 · Codex 1`, with `?` when evidence is stale. It does not claim to know which file an agent is editing. Pins remain visible after runtime disappearance; ordinary missing rows leave the inventory. Shared folders never establish shared conversation identity.

## What already exists

CX is a Rust terminal workspace with native tmux attachment, persistent shells and agent launches, scoped file previews/transfers, Docker/devcontainer access, SSH jump routes and signed updates. Its framing and scope protections are substantial. The opportunity is to connect those mechanisms into consistent session workflows.

| Mechanism | Actual coverage and limitation |
| --- | --- |
| Session discovery | Enumerates the CX-managed host tmux server and the execution user's default host tmux server. Custom sockets, inner-container tmux and arbitrary terminal sessions are not a unified inventory. |
| External session | Currently means a session outside the CX-managed host tmux server. It does not describe attachment mechanism, container scope or lifecycle permissions independently. |
| Container discovery | Lists Docker containers created by any tool. Labels or verified workspace configuration identify devcontainers; ordinary containers need explicit access enablement. |
| Container terminal | Host tmux wraps Docker/Dev Containers exec. Existing CX container sessions are attachable through Sessions, but container actions create new shells. |
| Agent launch and resume | Host UI supports fresh/latest resume. Container CLI/backend can launch installed agents; container UI offers only Shell and resume is not propagated through its launcher. |
| Jobs | Durable transfer jobs, not general build/evaluation runs with completion history. |
| Networking | Observations, enrollment and saved SSH routes exist. Managed service forwarding is absent; production sharing/reverse-access enforcement is incomplete. |

Evidence: `src/sessions.rs:40,560,1049,1205,1467`; `src/containers.rs:58,434,576,857,883`; `src/ui/menus.rs:34`; `src/main.rs:364,405,884`; `docs/containers.md`; `docs/robot-mesh-sharing.md`.

## Should every terminal start inside tmux

An opt-in persistent terminal mode is promising. A terminal launcher can start a new interactive shell inside tmux so work begun outside the CX UI is discoverable and reattachable later. It needs a clear bypass and should be tested before any shell-startup integration is offered.

A shell metadata hook serves a different purpose: it reports identity, current folder and limited activity information. Adding one to an existing shell does not generally make that shell's PTY reattachable. Existing tmux sessions can be joined through their actual endpoints; ordinary existing Docker exec or editor terminals cannot be advertised as recoverable merely because their processes are visible.

There is an immediate compatibility blocker: `sessions::attach` rejects a viewer with `TMUX` set (`src/sessions.rs:1468`). Wrapping every terminal today would make ordinary CX attachment fail from those terminals. Same-server client switching and return-to-CX can be investigated first; cross-server, remote and container handoff require distinct handling. Clearing the environment variable alone does not solve nesting.

The existing managed profile changes Ctrl+], the status line, mouse capture, scroll/copy behavior, clipboard forwarding and terminal identity (`assets/tmux.conf:5`; `src/sessions.rs:930`). Hiding the status line would not establish invisible behavior. Existing user-owned tmux servers must keep their configuration and bindings.

Recommended integration layers:

1. **Join existing persistent terminals.** Discover the default server and explicitly registered custom/container endpoints. Attachment does not grant lifecycle ownership.
2. **Optional shell metadata hooks.** Bounded, coalesced local events for prompt/directory/process identity. No network call or process scan on every prompt; no transcripts, full command lines or credential collection. Reconcile missed events on demand.
3. **Optional terminal launch profile.** Start new interactive terminals persistently, skip nested wrapping and noninteractive execution, preserve shell/login semantics, and provide an immediate bypass. Support launch arguments and environment deliberately rather than relying on a blanket shell startup snippet.

Initial prototype acceptance must include starting CX inside the wrapped terminal, switching and returning, two clients, exact runtime reattachment, normal shell exit versus terminal-window closure, Ctrl+C/Z/D, editor TUIs, Claude/Codex, scrolling, paste, selection, clipboard, resize and SSH disconnect. Noninteractive SSH commands, file transfers and scripts must remain unaffected. Compare launch-to-prompt and input latency with the ordinary terminal on the same machine. No universal wrapper or personal configuration changes were made in this exploration.

## First improvement cycle

Effort estimates are relative: S is a narrow change, M spans a workflow and its fixtures, L introduces a new lifecycle or attachment contract. They are not calendar estimates.

### 1 Unified session selection and attachment

**Value:** Return to the intended shell or agent quickly from a device, project or container. Show all matching sessions with device, scope, folder, provider and freshness. Attach and Watch act on existing work; New is explicit. Search should include container names as well as session/project labels.

Start with already-known sessions, preserving their exact identities. Container terminal actions currently always generate a new creation key (`src/ui.rs:1428,3020,3040`); host matching instead chooses the first matching session (`src/ui.rs:2569`). Both should lead to a clear choice among existing work. Container name is absent from the current session-search concatenation (`src/ui.rs:899`).

Then introduce registered attachment endpoints for custom host tmux sockets and in-container tmux. The current `external` boolean routes attachment to a host server and is insufficient for these additions (`src/sessions.rs:1467`). Keep execution location, attachment endpoint and lifecycle ownership separate. Inspecting or attaching external work must not automatically authorize stopping it.

**Acceptance:** Two sessions in one folder remain individually selectable; choosing Attach issues zero Create operations. Equal names/paths across hosts and containers cannot collide. A registered external tmux session retains its configuration and process identity. Plain unmanaged PTYs are honestly identified as unavailable for reattachment. First slice M; extended attachment coverage L.

### 2 Accurate session context and container agent parity

**Value:** The session list says where work is happening and exposes the agent choices actually available there.

Host process traversal does not generally establish the foreground agent inside a daemon-owned Docker exec process. Container sessions also report their saved launch folder rather than a proven current folder (`src/sessions.rs:117,235,507,538`). Extend the existing private container process tracker with bounded runtime observations; retain launch folder separately and show unknown when current state cannot be established.

Expose installed Codex/Claude profiles after an explicit capability probe for the selected container/user. The backend already supports them (`src/containers.rs:857`), while UI choices are shell-only (`src/ui/menus.rs:34`). Installation availability must not be described as provider authentication or readiness.

Wire fresh/native-resume intent through every container launch boundary. Both current UI/CLI force `resume: false`, and the container payload omits it (`src/ui.rs:1440`; `src/main.rs:906`; `src/sessions.rs:1205`; `src/containers.rs:883,1056`). A Resume button alone would not implement resume. Preserve explicit per-launch permissions; do not copy credentials or provider history automatically.

**Acceptance:** Shell → change folder → launch agent → exit agent produces truthful context. Host-installed tools do not appear available inside a container lacking them. Disposable synthetic providers verify exact fresh/resume/permission arguments. Effort M, with container observation/schema work coordinated by root.

### 3 Explicit lifecycle and recovery

**Value:** Offline work stays recognizable; ended work explains what happened and offers an intentional next action.

Current listing contains live tmux sessions and omits failed inspections (`src/sessions.rs:560`). Preserve bounded records distinguishing last observed running, unreachable, ended and container lifecycle changes. An unreachable host is not proof that a session ended. A completed process is not proof that an agent completed its task.

Offer Attach only when its endpoint remains valid. After container restart/rebuild, offer a new session in a freshly validated scope or provider-native resume when history is actually available. Retain old-runtime identity and use a new launch key; never silently follow a container name or reuse stale transfers/clipboard scopes. Existing engine/start/user checks remain mandatory (`src/containers.rs:605`).

Container discovery currently silently skips individual inspection failures (`src/containers.rs:590`). Add bounded per-entry issues and partial-result state so external work does not simply disappear without explanation.

**Acceptance:** Offline/reconnect preserves the same session; restart and same-name replacement cannot masquerade as continuation; ambiguous workspace matches refuse reassociation; missing provider history is explicit. Effort M/L.

### 4 Measure and remove interaction delays

Use a fixture benchmark before assigning causes to the reported slowness. The source exposes opportunities, but this pass did not measure end-to-end latency.

| Candidate | Source evidence | First experiment |
| --- | --- | --- |
| Obsolete navigation reads | Bounded per-host FIFO queue executes reads before stale replies are discarded (`src/ui.rs:7965,3295,3955`). | Rapidly request 20 locations through a delayed fake helper; execute only the in-flight read plus the final relevant pending intent. Preserve mutation order and fairness. |
| Reply delivery | UI polls terminal input for normally 100 ms, independently of helper completions (`src/ui.rs:8314,8484`; `src/ui/notifications.rs:96`). | Measure completion-to-frame delay; test a unified wake path and bounded reply processing with correct native-terminal handoff. Keyboard events themselves can wake the existing poll. |
| Repeated metadata subprocesses | Session inspection queries several tmux fields separately; Docker discovery inspects containers sequentially (`src/sessions.rs:67,476`; `src/containers.rs:576`). | Count subprocesses at 1/10/100 sessions; batch safely encoded snapshots and prioritize the selected scope. |
| Cross-container blocking | Owned helper map mutex spans the entire exchange (`src/transport.rs:283`). | Block one fake helper key and verify another can finish after changing to per-key locks, with safe eviction. |
| Preview redraw cost | Full-document cloning/wrapping/search work remains in redraw paths (`src/ui.rs:6308,6333,6432`). | Cache revision/width/query results; measure warm scroll frames without rescanning the document. |
| Transfer round trips | Synchronous 128 KiB read/write/ack sequence and repeated tree metadata reads (`src/transfers.rs:25,666,1020`). | Controlled endpoint delays; count requests and measure durable throughput before changing chunk/window semantics. |

Proposed starting budgets: cached selection/input-to-frame p95 under 33 ms, helper-completion-to-frame p95 under 16 ms, and no unbounded dependency on an unrelated slow host. Set achievable budgets against a nominated machine after measuring cold/warm p50/p95/p99, queue time, execution time, render time and memory. Terminal-wrapper overhead needs its own comparison. These numbers are targets, not observed results.

## High value new capabilities

| Priority | Feature | Useful first slice | Main tradeoff |
| --- | --- | --- | --- |
| 1 | Persistent workspace quick switcher | Recent/pinned device + scope + folder entries leading to sessions, Files and New; survives ordinary viewer exit. | A workspace persists longer than a container/runtime. Revalidate replacement scope before launching. M. |
| 2 | Git worktree launch contexts | Show branch/detached state and dirty status; select an existing worktree before launching an agent. | Observe Git without changing refs/index or weakening safe-directory checks. Explicit creation can follow; no automatic merges/cleanup. M. |
| 3 | Reliable automation lifecycle | Session lookup by ID/key, identity-checked stop, consistent CLI/container name/key/resume controls and structured outcomes. | Existing-key creation must compare immutable launch intent, not only permissions/scope. Resolve lost replies by lookup, never blind replay. M. |
| 4 | Persistent command runs and results | Explicit remote build/test/evaluation runs with real start/end/exit receipts and selected artifact paths; native attach/watch. | New lifecycle semantics. Keep interrupted/unknown distinct; avoid default transcript/log or command-secret retention. L. |
| 5 | Managed local service access | Explicit loopback SSH tunnel to a remote dev server, notebook or existing published container port, through the saved route. | Own only the forward; distinguish listener establishment from service health. No automatic exposure or network rewriting. M. |

These features support a later task-oriented agent workflow: choose an isolated checkout, launch work, reconnect, inspect results and retrieve artifacts. A fan-out agent orchestrator should follow reliable session identity and execution receipts. Adding broad automation first would multiply today's ambiguous state.

For native agent continuation, keep existing-runtime attach, provider-native conversation resume and fresh launch visibly distinct. CX runtime identity is not provider conversation identity. Do not infer exact threads from shared folders, titles or recency, or ingest transcripts to guess them (`research/codex-session-names.md`).

Production internet sharing, ROS relays, graphical application provisioning, broad new container runtimes and a VPN/mesh replacement are lower priorities for this cycle. Existing local networks and approved SSH routes remain the foundation. No doctor/dashboard or terminal emulator is proposed.

## Integration and verification

First demonstrate one end-to-end slice: **find existing work → attach/watch → return → select the same identity again**, across a host shell, an agent and an externally created container containing a CX terminal. Follow with registered external tmux endpoints and explicit nested-client handoff before the optional terminal launcher. Container agent parity and accurate runtime metadata can progress alongside those once root defines contracts.

Root owns `model`, `store`, `transport`, CLI dispatch, Cargo files and shared protocols. Define attachment endpoints and ownership separately before changing frozen session schemas. Add capability negotiation for new remote operations; old helpers must fail explicitly rather than silently downgrade resume or scope semantics. Keep initial picker/scheduling improvements wire-compatible where possible. Resolve the existing root integration conflicts before integrating implementation; this proposal does not resolve or overwrite that work.

Actual checks on the isolated snapshot:

- `cargo check --locked --offline`: passed; 33 existing dead-code warnings.
- `cargo test --locked --offline --bin cx ui::tests:: -- --test-threads=1`: 175 passed, 0 failed, 163 filtered out. Output: `/tmp/cx-astra-ideation-ui-tests.log`.
- `git diff --cached --check`: passed for the proposal and status changes.

The specialists performed source inspection only. Existing container, native PTY, discovery and reconnect fixtures were inspected, not rerun in this pass. No latency benchmark, physical terminal comparison, live container/SSH acceptance or new-feature implementation is claimed. The current UI checks do not validate the proposed attachment and recovery behavior.

At the first working review, demonstrate ordinary versus persistent terminal behavior and settle terminal-close versus explicit shell-exit semantics. Other costly contracts to validate are custom/container attachment identity, permissions for external lifecycle actions, provider-thread identity and workspace reassociation after rebuild. The user's priority is already settled: native-feeling session management across machines, including work started outside CX.
