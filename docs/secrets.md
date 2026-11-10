# Secrets

`cx secrets` keeps API tokens and keys out of plaintext files, shell history and argv. Values go into the OS keyring
when one answers, else into an age-encrypted file. A metadata index (names, dates, backend, length; never
values) lets `cx secrets list` work without touching either backend; it is still read from disk on each call.

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
`cx mesh` and `cx claude`. cx never reads or rewrites provider credential files (Claude/Codex logins).

## Backends

1. **OS keyring** (service `cx`, account = secret name): the Secret Service (GNOME Keyring, KWallet, KeePassXC) over
   a pure-Rust D-Bus client, or the macOS Keychain. It is tried only when a session bus exists
   (`$DBUS_SESSION_BUS_ADDRESS` or `$XDG_RUNTIME_DIR/bus`). Each call is bounded to 3 s; a keyring that does not
   answer (locked collection waiting on a prompt nobody sees, wedged dbus) makes `set` fall back to the file, and is
   not retried for the rest of that process. A secret stored in the keyring is readable only from sessions that reach
   that keyring; from a plain SSH login without a bus, `get` reports that instead of guessing.
2. **age file** `$XDG_STATE_HOME/cx/secrets.age` (default `~/.local/state/cx`), encrypted to an X25519 identity in
   `secrets.key` next to it. Both are 0600 in the 0700 state directory. Writes go to a temporary file, are fsynced and
   renamed into place, under a lock (`secrets.lock`). A corrupt or truncated file, or a missing identity, is reported
   and left untouched; cx never mints a new identity over an existing store.

`CX_SECRETS_BACKEND=file` never touches the keyring; `CX_SECRETS_BACKEND=keyring` refuses to fall back.

## Threat model

- The age file with its identity file is protected the way an SSH private key without a passphrase is: file modes
  keep other users out, and a copy of `secrets.age` alone (backup, sync folder, bug report) reveals nothing. It does
  **not** protect against root, against other processes of the same user, or against someone who copies both files.
  That is the deliberate trade-off for headless robots, which must start without a person typing a passphrase.
- The OS keyring adds what the platform gives: encryption at rest tied to the login password, and locking when the
  session locks. Any process of the same unlocked session can still ask it for the value.
- The index reveals names, timestamps and value lengths (for the masked `•••• (40)` display).
- Values live in memory in zeroize-on-drop buffers, and `SecretValue` has no `Display` and a redacted `Debug`.
  Copies made by the keyring library, the JSON parser or `std::process::Command` (which holds the environment it is
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
