# Exact provider session metadata

Investigated 2026-10-19. Installed providers: Claude Code 2.1.303 and Codex CLI
0.163.0. Read-only investigation inspected structured JSON key names, SQLite
schema, status enums and counts. No provider settings, sessions or transcripts
were modified or printed.

## Integration contract

`session_titles::native_metadata_with_title(provider, pid, selected_title)` returns
optional native title, current directory, activity and a bounded list of
`NativeAgentTask { id, name, task: Option<String>, status }`. Runtime code should
call this once per observed process and map into the shared `Session` fields.
Unknown metadata must clear previous derived metadata rather than preserve a stale
native title, directory or agent list. Keep explicit CX aliases separate from
native names. Tracked Codex launches clear the directory whenever selection or
directory evidence is unavailable; they never fall back to original launch cwd.

Claude's exact PID/start tick/session UUID record supplies current `cwd`, `name`
and `status`. Its `pidDomain`, where present, must also match this machine and PID
namespace. Name absence does not suppress valid directory/status metadata. The
same exact session UUID selects `.claude/tasks/<uuid>/<numeric-id>.json`. Task
`subject`, `owner` and `status` are structured display metadata; `description`
and transcripts are never used. Unassigned tasks are honestly labeled `Task N`.
Completed tasks remain visible. These are task assignments, not proof that every
owner is a currently running OS process.

Codex associates writable open thread-lock or rollout UUIDs owned by the same
live process. For uninstrumented processes, a singleton identifies that exact
thread; multiple writers require a proven common owned root in the spawn-edge
graph. Independent roots, cycles and incomplete traversals remain unavailable.
Tracked launches additionally require the controlled native selection signal,
even with one writer; see the actual TUI evidence below. Read-only historical handles are excluded; no
folder/recency guessing. Once the root UUID is proven, an optional read-only `sqlite3` subprocess selects ONLY `name`,
`cwd`, child IDs, nicknames, roles and spawn-edge status from `state_5.sqlite`.
`title`, `preview`, `first_user_message`, rollouts and turns are excluded. In this
schema `title` may contain initial user content, so it is intentionally never a
name fallback. A bounded recursive query follows actual `thread_spawn_edges`
to depth eight and at most 128 descendants. The native append-only name index
still takes precedence for fresh rename updates. Child task text is its native
name or role, and can therefore be less descriptive than its full private prompt.

`sqlite3` is an optional host executable, present on the investigated host.
Missing executable, schema change, unreadable database, timeout or bad records
produce unavailable optional metadata. The compatibility query is version
sensitive, not a public stable provider API. Process-start identity is rechecked
after collection. Reads/records are capped; SQLite is killed after 150 ms and
output capped at 256 KiB. Multiple-writer ancestry resolution has a separate
150 ms subprocess budget, at most 64 owned writers, depth eight and 8192 rows.
SQLite startup commands are disabled with `-init /dev/null`; the user
`.sqliterc` is never sourced. The native rename index is collected first using
a fresh 100 ms budget, preserving its precedence even if SQLite is slow. Every task/metadata file rejects symlinks, nonregular
files and different file owners. Display strings remove terminal/bidi controls.

## Evidence and remaining limits

Targeted Rust tests cover PID reuse, no-name directory/activity metadata,
exact renames, writer ambiguity, control sanitization, record/time limits, task
ID mismatch/symlink/oversize rejection, prompt-field exclusion, and an actual
read-only SQLite fixture with parent, child, grandchild and unrelated same-cwd
thread. The SQLite fixtures confirm unrelated threads never enter the tree,
parent/child/grandchild writers select one root, unrelated roots/cycles fail
closed, oversized ancestry is rejected and `.sqliterc` output is suppressed.

The official [Codex App Server documentation](https://developers.openai.com/codex/app-server/)
was fetched on the investigation date. It documents thread reads, names and
parent thread relationships. It does not establish a terminal/PID to selected
thread association. Official-domain search tooling returned an error; the
matching official page was fetched directly. The local database adapter is
explicitly based on observed installed schema, not claimed as a documented API.

A shared-daemon TUI without an exact writable thread handle remains unknown.
Native Codex activity is also unknown: spawn-edge `open` means an edge remains
open, not that its child is actively computing. Do not render `open` as busy.
Claude recognizes only a small status vocabulary, preserving unknown as absent.
Runtime's separate observed-command indicator can provide shell activity without
claiming inference is underway.

An isolated actual Codex 0.163.0 TUI test established that idle resume switching
retains old writable thread handles even with `--no-daemon`. Per-launch lifecycle
hooks also defer until a turn, so no hook is shipped. New supported host launches
instead use the per-launch `tui.terminal_title=["thread-name"]` option and a
managed-launch marker. The controlled native signal must match exactly one
explicit name among this process's owned writer IDs. All candidates must have
lossless names of at most 48 characters; missing, duplicate, sanitized or
truncated names fail closed. No generic title or cwd/recency fallback is used.

`tests/codex_native_switch_live.py` passes against actual Codex 0.163.0: resume
another synthetic project, choose “Use session directory”, and observe the new
name/directory with stable terminal identity and no inference on the idle switch.
Private provider configuration and a rejecting loopback API isolate the test
from actual accounts and conversations. Ambiguity clears the directory instead
of retaining a misleading launch path. Existing processes remain untouched;
shared-daemon association and other provider versions remain limitations.
