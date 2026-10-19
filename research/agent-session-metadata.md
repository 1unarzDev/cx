# Exact provider session metadata

Investigated 2026-10-19. Installed providers: Claude Code 2.1.303 and Codex CLI
0.163.0. Read-only investigation inspected structured JSON key names, SQLite
schema, status enums and counts. No provider settings, sessions or transcripts
were modified or printed.

## Integration contract

`session_titles::native_metadata(provider, pid) -> Option<NativeMetadata>` returns
optional native title, current directory, activity and a bounded list of
`NativeAgentTask { id, name, task: Option<String>, status }`. Runtime code should
call this once per observed process and map into the shared `Session` fields.
Unknown metadata must clear previous derived metadata rather than preserve a stale
native title, directory or agent list. Keep explicit CX aliases separate from
native names. Directory fallback can use observed process cwd, but the original
launch `--cd` must not override proven current native directory.

Claude's exact PID/start tick/session UUID record supplies current `cwd`, `name`
and `status`. Its `pidDomain`, where present, must also match this machine and PID
namespace. Name absence does not suppress valid directory/status metadata. The
same exact session UUID selects `.claude/tasks/<uuid>/<numeric-id>.json`. Task
`subject`, `owner` and `status` are structured display metadata; `description`
and transcripts are never used. Unassigned tasks are honestly labeled `Task N`.
Completed tasks remain visible. These are task assignments, not proof that every
owner is a currently running OS process.

Codex associates writable open thread-lock or rollout UUIDs owned by the same
live process. A singleton identifies that exact thread. With multiple owned
writers, the actual spawn-edge graph must prove exactly one owned root is an
ancestor of every other owned writer. Independent roots, cycles and incomplete
traversals remain unavailable. Read-only historical handles are excluded; no
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

Fourteen targeted Rust tests cover PID reuse, no-name directory/activity metadata,
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
claiming inference is underway. No hook was installed and no inference request
was sent to manufacture evidence. No end-to-end interactive provider-switch
validation is claimed by this module. A durable shared-daemon identity bridge
still needs trusted native hooks or upstream client registration; see the earlier
[identity research](codex-session-names.md).

The owned-writer graph is structural identity, not proof of TUI selection
lifecycle. No read-only observation can establish that switching the interactive
TUI to another thread releases the old root's writer. A disposable idle
`--no-daemon` TUI folder/thread-switch test is still required before claiming
end-to-end current-directory tracking. Multiple unrelated writers safely become
unavailable. A shared daemon owning a single tree cannot independently identify
which child a particular TUI selected; separate managed processes avoid the
shared-client association problem but do not replace that lifecycle test.
