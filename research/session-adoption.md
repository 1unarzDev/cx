# Session adoption: reptyr spike (M0)

Date: 2026-10-19. Host: Arch Linux, kernel 7.2.9, tmux 3.8, Yama `ptrace_scope=1`.
reptyr 0.10.0 built from the upstream tag tarball
(`sha256 c6ffbc34a511ac00d072219bda30699e51f2f4eb483cbae9e32e981d49e8b380`).

## Method

Ptrace was granted without host changes by running the harness inside `unshare -Ur`.
Yama accepts `CAP_SYS_PTRACE` in the target's user namespace. The "denied" case ran
without a user namespace under the host's scope 1.

The target was a synthetic churn stub. It:
- writes to the tty every 10 ms;
- runs four busy threads and a forked child that writes to a pipe (the "tool");
- echoes stdin;
- logs SIGHUP, SIGWINCH, SIGTERM and SIGCONT, EIO, stdin EOF, the tty path and the largest gap between iterations.

Holders tested:
- a multithreaded Python "emulator" that owns the PTY master;
- a foreign `tmux -L` server.

Shapes tested: the stub as session leader, and the stub as a foreground job under `bash -i`.
reptyr ran inside a pane of a second tmux server, standing in for cx's managed server.

## Results

| Case | Result |
|---|---|
| `-T`, stub as leader, emulator holder | Adopted. Output reached the new pane. Input and resizes (120x40, 100x35) reached the target. Largest output gap 10 ms. Threads and the tool child were unaffected. |
| `-T`, stub as job under bash | Same as above. The whole terminal session moved, including bash, so the prompt returns in cx when the agent exits. |
| `-T`, foreign tmux holder | Adopted. The foreign pane stays alive but frozen on its last frame. |
| Per-process mode | Refused: "Process N shares N's process group. Unable to attach." Any target with children in its process group is refused, which describes every real agent. |
| ptrace denied | reptyr exits 1 with "Operation not permitted". The target, its tty and the old pane are untouched. |
| Original window or foreign tmux server closed after adoption | The target survives, because the relay keeps its own reference to the master. |
| Relay (reptyr pane) killed | The target receives EIO and stdin EOF, with **no SIGHUP**. A process that ignores EIO keeps running on a dead terminal. |

In every `-T` case the target's fd 0 path does not change. The target keeps its original
`/dev/pts/N`, and reptyr relays between that stolen master and the pane.

## Decisions for M1–M3

- **Use `-T` (steal the terminal) only.** Drop per-process mode. Adoption therefore requires
  ptrace on the master holder (emulator, sshd session process, tmux or screen server), not on
  the agent itself. For a `docker exec -it` client, the host terminal is stolen and the CLI moves with it.
- **Success means the relay is live.** The checks are: a cx-reptyr process exists in the pane,
  the target and holder identities are unchanged, and output arrives in the pane. Do not
  check for "fd 0 moved", because it never moves.
- **Relay death is a hangup cx owns.** When cx stops a session, resets, or finds the relay dead,
  it must SIGHUP and then SIGTERM the adopted session's process group after re-verifying identity.
  It must never leave agents spinning on a dead terminal.
- **Failure leaves the original intact.** Denied or refused attaches keep the target on its
  original terminal, so the fallback (M4) can follow safely.
- **Watch the old holder.** A frozen foreign tmux pane or emulator window remains after adoption.
  Its controller (`tmux`, `screen`) can be left as is. Document this.

## Not yet established

- A real sshd session holder (privilege-separated, runs as the user).
- GNU screen (not installed here).
- VTE- and Electron-based emulators.
- Real Claude and Codex mid-turn.
- logind `KillUserProcesses=yes`.

These belong to the M3 lifecycle fixtures and to manual acceptance with the user.

## Guarded relay findings (M2/M3)

- **The attach in upstream reptyr 0.10.0 races.** It calls `PTRACE_ATTACH` and then
  `kill(pid, SIGCONT)` straight away. If the SIGCONT arrives before the tracee takes the
  queued SIGSTOP, the kernel throws the SIGSTOP away and `waitpid` blocks forever.
  - Observed in 1 of 12 runs against the multithreaded churn stub. Every case had a
    `signal CONT` in the target's log.
  - The patch attaches with `PTRACE_SEIZE` + `PTRACE_INTERRUPT` instead.
  - With the patch: 30/30 guard runs adopted in under 0.1 s, and 8/8 full live fixture
    runs passed.
- **The session leader keeps ignoring SIGHUP after a steal.** `steal_block_hup` sets SIGHUP
  to ignored in the leader, permanently. CX therefore stops adopted terminals with SIGTERM
  to each process group of the session, sent while the relay is still live. It then closes
  the relay, which gives interactive shells EOF.
- **The guard narrows what cx-reptyr will do.** It accepts only
  `-T TARGET START LEADER START HOLDER START`. All three processes must belong to the
  caller's real uid/gid (real, effective, saved and fs ids, and the `/proc` owner), and
  their start times must still match. The leader's parent must be the holder.
  Identities are checked again after each ptrace attach. The relay runs with
  `CapEff=0` once the steal is done.
- **A process that ignores SIGTERM survives a stop.** This matches managed-session stop,
  which only hangs up. CX does not escalate to SIGKILL.
