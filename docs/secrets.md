# Secrets

`cx secrets` keeps API tokens and keys out of plaintext files, shell history and argv. Values go into the OS keyring
when one answers, else into a private file stored the way `ssh-keygen` stores an unencrypted private key. A metadata index (names, dates, backend, length; never
values) lets `cx secrets list` work without touching either backend; it is still read from disk on each call.

The hub's keys screen (`K`) sets and rotates the same secrets in a masked entry; the value is never drawn or
logged, and its buffer is wiped when the entry closes. There `e` exposes the selected secret to new sessions and `v`
names its variable ([Sessions](#sessions)).

```
cx secrets list [--json]                     # metadata only
printf %s "$TOKEN" | cx secrets set NAME
cx secrets set NAME                          # on a terminal: prompts with echo off
cx secrets get NAME --fd 3 3>file            # write the value to an inherited descriptor (never a terminal)
cx secrets get NAME [VAR=NAME...] --exec -- cmd args   # run cmd with the values in its environment only
cx secrets rm NAME
cx secrets migrate [--delete-plaintext]      # import the `cx mesh` token file as cloudflare.api_token
```

Values are never accepted on the command line and never printed to a terminal. `get --fd 1` works when stdout is a
pipe (`token=$(cx secrets get x --fd 1)`) and is refused when it is a terminal.

## Names and variables

Names are 1-64 characters of `a-z 0-9 . _ -`. `get --exec` sets the name upper-cased with `.`/`-` as `_`
(`robot.wifi` → `ROBOT_WIFI`) unless the spec says `VAR=NAME`, and refuses two specs that set one variable. The one
name cx itself reads is `cloudflare.api_token`, which sets `CLOUDFLARE_API_TOKEN` and is passed to mesh.py by
`cx mesh` and `cx claude`. The same rule gives the conventional names for sessions: `anthropic.api_key` →
`ANTHROPIC_API_KEY`, `openai.api_key` → `OPENAI_API_KEY`, `github.token` → `GITHUB_TOKEN`. cx never reads or rewrites provider credential files (Claude/Codex logins).

## Sessions

Each secret has an **expose to sessions** switch, off by default: `e` on its card in the keys screen turns it on or
off, and the card then reads `used by new sessions as $VAR`. `v` edits that variable (empty restores the default
above); it must be a valid name, not `CX_*` (cx sets those in sessions itself), and not one another exposed secret
already sets. Both are stored in the index (`expose`, `env`), never with the value.

A shell, Claude or Codex session that cx starts on this device afterwards, from its viewer or `cx new`, starts with
every exposed secret in its environment. Sessions that already run keep theirs: turning a secret on or off, or
rotating it, changes only sessions started later. What never gets them:

- **Other devices.** A session cx starts on another device is created there by that device's `cx helper`; the request
  carries no secret, and the helper never exposes its own device's secrets to it either (they are for sessions
  started at that device). Copy a secret to another device with `cx secrets set` there.
- **Containers**, command panels (`:`) and adopted terminals.

How the value gets there: tmux has no way to hand one new pane an environment variable without keeping it. Variables
of the `tmux new-session` client do not reach the pane, and `set-environment` or `new-session -e` store the value in
the server, where `show-environment` prints it and every later pane inherits it. So the pane's command is
`cx secrets get --dir STATE VAR=NAME… --exec -- <the launcher>`: that cx reads the values from the store itself and
`exec`s the launcher with them in its environment, the same pid tmux started. tmux, the pane's start command and
`/proc/PID/cmdline` see only names. Before starting it, the launching cx reads each exposed value once, so a locked
keyring or a missing value is reported as the launch's error instead of a session that exits at once. `--dir` names
the state directory of the launching cx, because the managed tmux server runs as its own user service, whose
environment may differ.

Inside the session the values are ordinary environment variables: every program started there inherits them, and the
same user can read them from `/proc/PID/environ`, as with `--exec`. Expose only what you would type into that shell.

## Backends

1. **OS keyring** (Linux): the Secret Service (GNOME Keyring, KWallet, KeePassXC) through libsecret's `secret-tool`,
   with the attributes `service=cx name=<secret>`. The value goes to `secret-tool store` on its stdin and comes back
   from `secret-tool lookup` on its stdout; argv only ever holds the name. cx links no D-Bus or keyring library. The
   keyring is tried only when a session bus exists (`$DBUS_SESSION_BUS_ADDRESS` or `$XDG_RUNTIME_DIR/bus`) and
   `secret-tool` is on PATH (on Debian and Ubuntu, package `libsecret-tools`). Each call is bounded to 3 s; a
   keyring that does not answer (a locked collection waiting on a prompt nobody sees, a wedged bus) makes `set` fall
   back to the file, is not retried for the rest of that process, and its `secret-tool` process group is killed a
   second later. A secret stored in the keyring is readable only from sessions that reach that keyring; from a
   plain SSH login without a bus, `get` reports that instead of guessing. `secret-tool lookup` exits 1 both for a
   missing entry and for some keyring failures, so cx reads both as "not in the keyring".
   **macOS** uses the fallback file: `security add-generic-password` takes the password only in argv (`-w`, which
   any local process can read from the process table) or at an interactive prompt, so cx does not use it.
2. **Fallback file** `$XDG_STATE_HOME/cx/secrets.values.json` (default `~/.local/state/cx`): the values as plain
   JSON, 0600 in the 0700 state directory. It is opened without following symlinks and refused unless it is a
   regular file owned by this user with no group or other permission bits. Writes go to a 0600 temporary file, are
   fsynced and renamed into place, under a lock (`secrets.lock`). A corrupt or truncated file is reported and left
   untouched.

`CX_SECRETS_BACKEND=file` never touches the keyring (`auto`, the default, uses it when it answers).

## Threat model

- The fallback file is exactly as safe as `~/.ssh/id_ed25519` without a passphrase, and no safer: file modes keep
  other users out, and nothing else does. Root, any process of the same user, and anyone who gets a copy of the file
  (backup, sync folder, bug report) can read the values. Encrypting it with a key kept next to it would not change
  that, so cx does not (an earlier draft used age and was dropped for its dependency weight). That is the
  deliberate trade-off for headless robots, which must start without a person typing a passphrase; put the state
  directory where you would put SSH keys, and prefer the keyring where one exists.
- The OS keyring adds what the platform gives: encryption at rest tied to the login password, and locking when the
  session locks. Any process of the same unlocked session can still ask it for the value.
- The index reveals names, timestamps and value lengths (for the masked `•••• (40)` display).
- Values live in memory in zeroize-on-drop buffers, and `SecretValue` has no `Display` and a redacted `Debug`.
  Copies made by the pipe from `secret-tool`, the JSON parser or `std::process::Command` (which holds the environment it is
  given) are not zeroized; this is best effort, not a guarantee against memory inspection.
- `--exec` places values in the child's environment, which the same user can read via `/proc/PID/environ` while it
  runs; argv (`/proc/PID/cmdline`, world-readable) never carries them.
- Deleting the plaintext file after `migrate` unlinks it; it does not scrub the disk blocks.

## mesh.py

mesh/mesh.py still works standalone: `$CLOUDFLARE_API_TOKEN`, then its `credentials` file, then a prompt. `cx mesh`
and `cx claude` set `$CLOUDFLARE_API_TOKEN` from `cloudflare.api_token` when it is stored and the variable is not
already set, and only for subcommands that call the Cloudflare API:

- `cx mesh setup | invite | devices | deploy | audit | profile | revoke | teardown`
- `cx claude lend | revoke | list`

The stored token belongs to the default mesh (the `credentials` file `migrate` imports): an argv with `--config`
gets no token, so that mesh keeps the `credentials` file next to its config. If a token is stored but cannot be read
in this session (keyring-held and no session bus, or a locked or hung keyring), `cx mesh` stops with that error
rather than letting mesh.py prompt and save the token in plaintext again; export `$CLOUDFLARE_API_TOKEN` there instead.

The token is an account admin token (it can mint API tokens and edit Zero Trust), so it is kept away from anything
that runs foreign code:

- `cx mesh join` and `leave` never get it. They pipe a script fetched from a URL (possibly another mesh's) into
  `sh`; mesh.py also removes `$CLOUDFLARE_API_TOKEN` from the environment of that `curl` and `sh`, so an exported
  token does not reach the script either, standalone or under cx.
- An argv whose subcommand cx cannot read with certainty (an unknown or abbreviated option before it, such as
  `--conf X`, `-h`, `--`) gets no token; spell options out (`--config X`, `--json`) or use `--exec` below.
- claude.py runs the `[claude] gateway` command without `$CLOUDFLARE_API_TOKEN` in its environment.

The remaining exposure: the mesh.py and claude.py processes themselves hold the token (in their environment, readable
by the same user via `/proc/PID/environ`), as they would after reading the credentials file; neither spawns anything
else that inherits it. After
`cx secrets migrate --delete-plaintext`, standalone runs need the variable (for example
`cx secrets get cloudflare.api_token --exec -- python3 mesh/mesh.py devices`).

## Prompt

On a terminal, `set` reads the value byte by byte with echo, line mode and terminal signals off: Ctrl-C (or
Ctrl-\) cancels without storing, Backspace (one character) and Ctrl-U edit, Enter or Ctrl-D ends. SIGINT, SIGTERM,
SIGHUP or SIGQUIT sent while the prompt waits restore the terminal mode before the process dies, so the shell is
never left with echo off.

## Keyring copies left behind

When a secret held in the keyring is replaced from a session that cannot reach the keyring (or while it fails), the
new value goes to the file and the old keyring copy is noted in the index (`stale_keyring`, names only). So is any
name whose keyring write failed or timed out just now, since a timed-out call may still land later. The next
`set` or `rm` from a session whose keyring answers deletes those copies.
