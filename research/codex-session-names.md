# Codex session names: identity before title lookup

Investigated 2026-10-07. Scope: research and design, without installing hooks,
changing provider configuration, renaming terminals, reading transcripts, or
sending inference requests. Local `codex --version` reports **0.161.0**. Source
claims below are pinned to its release commit
[`979011409de0a60b52f179721948e65531d26144`](https://github.com/openai/codex/tree/979011409de0a60b52f179721948e65531d26144).
Current official documentation was fetched; documentation can describe a
different build and should not override version-specific evidence.

## Finding

Codex has native names. CX's missing piece is identifying **which current Codex
thread belongs to a terminal**, especially when multiple terminals use the same
daemon. Looking up more names cannot repair a missing association.

There are two different name fields. `Thread.name` is the user-facing conversation
title. `Thread.agentNickname` is an optional nickname for an AgentControl-spawned
subagent, with a separate `agentRole`. It is not the general name of every CLI
session. The protocol also distinguishes thread ID, parent thread, and session-tree
ID. A terminal showing a parent conversation must not be relabeled after whichever
child happens to be active. See the [release Thread schema][thread].

Official [App Server documentation][app-server] documents `thread/name/set` and
`thread/name/updated`. `thread/read` reads without resuming or subscribing, and
omitting/setting false `includeTurns` avoids full turns. Names are hydrated on
`thread/list` and `thread/read`; new threads may have null names. These calls do
**not** supply a documented terminal/PID-to-current-thread association. A summary
still contains `preview`, which the [schema][thread] describes as usually the first
user message. Thus `includeTurns: false` does not make a response metadata-only.
CX must not ingest these summaries as a workaround under its no-transcript
constraint; even suppressing preview in logs would not prevent receiving it.

## What earlier attempts actually established

The repository's [session title code](../src/session_titles.rs) associates Codex
only with one exact writable rollout or writer-lock UUID opened by the observed
process, then finds that UUID in a bounded tail of `session_index.jsonl`. It rejects
multiple writers, incomplete scans, read-only historical handles and reused PIDs.
This is intentionally conservative. A foreground TUI connected to a shared daemon
need not own its rollout, and scanning the daemon finds multiple threads rather
than identifying the TUI's selected thread.

[STATUS.md](../STATUS.md), sections “Session readability — implementation
milestone” and “Session labels and empty-server refresh — v0.1.22 candidate,”
records successful synthetic writer/rename/ambiguity fixtures, working native
`/rename` on Codex 0.160.1, but no live daemon terminal association. It also records
three earlier read-only investigations reaching the same limitation. These are
not successful end-to-end native naming tests. The existing exact-writer reader
can remain a compatibility path; extending it to daemon-wide folder/recency
matching would introduce wrong labels.

The same status history records rejection of inherited pane-title fallback.
An old shell, editor, another foreground program or earlier Codex thread can have
written that title. Current implementation also preserves explicit or historically
non-default CX labels, whose provenance is unknown. Even successful native lookup
does not necessarily replace such labels. A naming implementation needs an
explicit precedence contract, preferably native title in the primary display
when association is proven and a separately preserved user alias.

## Approaches and tradeoffs

| Approach | Benefit | Limitation / suitability |
| --- | --- | --- |
| Existing exact writable FD + name index | No hooks; already bounded and covered by fixtures | Works only where the foreground process owns exactly one thread writer; does not solve shared-daemon TUI association |
| Daemon `thread/list` / `thread/read` | Supported native title API | No documented terminal association; receives prompt preview; cwd, update time, active state, list order and sole apparent match do not establish identity |
| Trusted `SessionStart` hook + per-launch marker | Can publish the real thread ID separately from title lookup | Native hook trust review and propagation/timing need a disposable test; applies to instrumented launches, not retroactively to arbitrary existing TUIs |
| Native terminal-title output | TUI emits the title of its selected thread, avoiding daemon FD association | Requires proven fresh output from the same managed launch; inherited pane titles are insufficient; display output can be truncated and contain progress decoration |
| Launch with an explicit known thread / client instrumentation | Bind the terminal to the ID returned/selected by that client; strongest association | Native TUI switching, `/new`, resume, fork and reconnect must update the binding; merely parsing initial `codex resume <id>` becomes stale |
| Upstream metadata-only terminal registration | A native client can report its current thread and name with PID/start identity | Best durable contract, but requires Codex support or a maintained native-client patch; not established by the current public schema |

### Hook prototype: viable, still unvalidated

Official [hooks documentation][hooks] says non-managed hooks must be reviewed and
trusted before execution. Hooks load additively; plugin hooks use the same trust
flow. Do not silently approve a handler or alter user's global provider config.
The [release SessionStart schema][hook-schema] has `session_id`, source
(`startup`, `resume`, `clear`, `compact`, `fork`), and a `transcript_path` that an
identity handler should ignore. [SessionStartRequest][hook-event] types the ID as
`ThreadId`. Subagent hooks require separate treatment and must not overwrite the
parent's terminal mapping.

A prospective CX launch can provide a random association marker through a
per-launch configuration overlay. [Official configuration documentation][config]
and [release policy source][env-policy] support `shell_environment_policy.set`.
The [environment builder][env-builder] adds `CODEX_THREAD_ID` to shell-like tool
environments. This does **not** imply it exists in the foreground TUI's `/proc`
environment. Arbitrary inherited launch variables are also not proof of daemon
forwarding. The [hook command runner][hook-runner] replays a session environment
snapshot and hook-specific overrides: the marker must be shown to arrive in that
snapshot in a real shared-daemon launch, with another simultaneous launch proving
no leakage.

Important timing caveat: [core hook runtime][hook-runtime] implements
`run_pending_session_start_hooks(sess, turn_context)` by draining pending start
events. This is not evidence that a no-prompt TUI immediately publishes identity
at startup. Do not send a paid/user turn just to obtain a name. Verify idle startup,
resume and thread switching without inference first. If identity arrives only
with a later natural turn, keep an honest fallback until then.

The fixed handler should write only marker + current thread ID to private bounded
CX state, atomically, with process/start/boot/session identity validation and
event ordering protection. It should emit no stdout context, ignore transcript
paths, and avoid shell interpolation of names/IDs. Resolve native names only by
the already-proven exact UUID, using the existing bounded name index where its
format is supported. Hook installation and launch instrumentation are integration
work, not an outcome of this research.

### Terminal-title prototype: simpler but weaker identity

The [release TUI configuration][tui-config] defaults `terminal_title` to activity,
thread-name and project-name. [Status rendering][status-surfaces] reads the selected
thread's native name; `thread-name` does not substitute a raw thread ID. Its title
part is truncated to 48 characters and can show title-generation progress.
[OSC output][osc] emits sanitized OSC 0 on a terminal and clears managed output
on exit. This is native TUI output, not chat-screen scraping.

The fetched official [configuration reference][config] describes different defaults
(`spinner`, `project`), reinforcing the need to test the actual installed release.
A launch-only `tui.terminal_title=["thread-name"]` overlay is worth a disposable
test without touching global configuration. Never enable blind `pane_title`
fallback: require a fresh same-launch output marker and foreground identity,
clear/expire on exit or thread transition, preserve manual aliases, and sanitize.
Parsing this as a display label still does not provide a durable thread ID.

## Recommended next step and acceptance gates

Keep existing sessions intact. For future CX-launched sessions, first prototype
an explicit native identity bridge: trusted fixed SessionStart handler plus a
per-launch marker, with exact-UUID name lookup. If idle startup/switch events cannot
establish identity without inference, prefer native-client/upstream registration
over more heuristics. Terminal-title observation can be an explicitly bounded
display experiment, not evidence that daemon thread association is solved.

Before shipping naming, validate in an isolated disposable session:

1. Two Codex TUIs sharing a daemon and cwd, with distinct native names; each must
   retain its own name through reordered metadata and opposite activity states.
2. Idle startup without a prompt, resume, clear/new, fork and thread selection:
   mapping must update or immediately become unknown; no stale rename allowed.
3. Native rename and blank/unset name, alias preservation, malformed/control text,
   duplicate/out-of-order hook delivery, PID reuse and old binding expiry.
4. Two instrumented launches with different markers; parent/subagent isolation,
   trusted/untrusted hook behavior, existing hooks remaining intact.
5. On each execution host, a bounded local helper lookup; no remote home-path
   assumptions, no agent forwarding, no transcript/preview ingress.

No runtime naming fix or end-to-end hook/OSC validation is claimed. Actual checks:
local version lookup; bounded repository reads; fetched official docs and
version-pinned first-party source/schema comparisons. No Cargo tests were needed
for this documentation-only change. The official-domain search tool returned an
error; matching official pages were fetched directly before inspecting CX code.

[app-server]: https://developers.openai.com/codex/app-server/
[hooks]: https://developers.openai.com/codex/hooks/
[config]: https://developers.openai.com/codex/config-reference/
[thread]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/app-server-protocol/schema/typescript/v2/Thread.ts
[hook-schema]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/hooks/schema/generated/session-start.command.input.schema.json
[hook-event]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/hooks/src/events/session_start.rs
[hook-runtime]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/core/src/hook_runtime.rs
[hook-runner]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/hooks/src/engine/command_runner.rs
[env-policy]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/config/src/shell_environment_policy.rs
[env-builder]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/protocol/src/shell_environment.rs
[tui-config]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/config/src/types.rs
[status-surfaces]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/tui/src/chatwidget/status_surfaces.rs
[osc]: https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/tui/src/terminal_title.rs
