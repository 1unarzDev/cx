#!/usr/bin/env bash
# Build cx and run the offline regression set described in tests/REGRESSIONS.md.
#
#   tests/run_regressions.sh [--quick] [--no-rust] [--with-ssh] [--with-docker] [--only NAME...] [--list]
#
# Default: cargo fmt --check, the full locked Rust suite (it holds most unit
# guards), then every Python fixture that needs only this machine: tmux,
# python3 and, for viewer/PTY screens, pyte. Nothing contacts a real device.
#   --quick        Rust: only the black-box regression binary and the updater
#                  member guard instead of the whole suite.
#   --no-rust      Skip cargo fmt/test (still builds target/debug/cx).
#   --with-ssh     Also run fixtures that start a disposable loopback sshd.
#   --with-docker  Also run fixtures that create owned network-none containers.
#   --only NAME    Run only the named Python fixtures (basename, no .py).
#   --list         Print the selected fixtures and exit.
# pyte: if `import pyte` fails, set CX_PYTHONPATH to a directory holding pyte
# and wcwidth (for example an unpacked wheel cache); otherwise pyte fixtures
# are reported as SKIP, never as PASS.
#
# Safety: TMUX/TMUX_PANE are unset and TMUX_TMPDIR points at a private
# directory for every step, so a fixture that runs bare `tmux` (or
# `tmux kill-server`) can never reach the caller's managed or personal tmux
# server, even when this script is started from inside a cx pane.
set -u
cd "$(dirname "$0")/.."
ROOT=$(pwd)

quick=0 rust=1 ssh=0 docker=0 list=0
only=()
while [ $# -gt 0 ]; do
    case "$1" in
    --quick) quick=1 ;;
    --no-rust) rust=0 ;;
    --with-ssh) ssh=1 ;;
    --with-docker) docker=1 ;;
    --list) list=1 ;;
    --only)
        shift
        while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do only+=("$1"); shift; done
        continue
        ;;
    -h | --help) sed -n '2,24p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

# name|requirements|arguments (BIN = target/debug/cx). Requirements are
# space-separated: pyte, fish, poppler, sshd, docker.
CORE=(
    "command_panel_live||BIN"
    "focus_exits_copy_mode_pty||BIN"
    "mouse_scrollback_pty||BIN"
    "orphan_exit_pty||BIN"
    "terminal_startup_readiness_live||BIN"
    "session_runtime_live||BIN"
    "container_discovery_fixtures||BIN"
    "container_session_reconcile_live||BIN"
    "session_activity_timestamp_live||BIN"
    "native_runtime_metadata_live||BIN"
    "codex_process_mode_live||BIN"
    "session_names_live||BIN"
    "install_fixture||"
    "device_policy_cli||BIN"
    "codex_wheel_pty||BIN"
    "codex_wheel_threads_pty||BIN"
    "clipboard_pty||BIN"
    "metadata_reconnect_cli||BIN"
    "tmux_discovery_live||BIN"
    "preview_helper_live||BIN"
    "pdf_pages_helper|poppler|BIN"
    "native_command_pty||BIN"
    "native_command_pty@sh||BIN"
    "native_command_pty@fish|fish|BIN"
    "stop_shell_live||BIN"
    "session_permissions_live||BIN"
    "manual_agents_live||BIN"
    "shell_labels_live|fish|BIN"
    "login_terminal_pty||BIN"
    "attach_terminal_pty||BIN"
    "folder_command_ui_pty||BIN"
    "mesh_util||"
    "session_chooser_pty|pyte|BIN"
    "startup_directory_pty|pyte|BIN"
    "terminal_startup_loader_pty|pyte|BIN"
    "transfer_enter_pty|pyte|BIN"
    "source_preview_pty|pyte|BIN"
    "markdown_preview_pty|pyte|BIN"
    "markdown_blocks_pty|pyte|BIN"
    "markdown_prose_pty|pyte|BIN"
    "markdown_relative_pty|pyte|BIN"
    "preview_search_pty|pyte|BIN"
    "browser_find_pty|pyte|BIN"
    "list_wheel_pty|pyte|BIN"
    "pdf_pages_pty|pyte poppler|BIN"
)
SSH=(
    "jump_routes_cli|sshd|BIN"
    "access_routes_cli|sshd|BIN"
    "peer_pairing_cli|sshd|BIN"
    "peer_mesh_cli|sshd|BIN"
    "peer_managed_ssh|sshd|BIN"
    "enrollment_tooling_cli|sshd|BIN"
    "enrollment_auth_pty|sshd pyte|BIN"
    "enrollment_ssh|sshd|."
    "robot_enrollment|sshd|"
)
DOCKER=(
    "containers_live|docker|BIN"
    "container_agents_live|docker|BIN"
    "container_transfers_live|docker|BIN"
    "devcontainer_terminal_recovery_live|docker|BIN"
    "claude_contrast_live|docker|BIN"
    "transfer_enter_pty@container|docker pyte|BIN --container"
)

selected=("${CORE[@]}")
[ $ssh = 1 ] && selected+=("${SSH[@]}")
[ $docker = 1 ] && selected+=("${DOCKER[@]}")
if [ ${#only[@]} -gt 0 ]; then
    filtered=()
    for entry in "${CORE[@]}" "${SSH[@]}" "${DOCKER[@]}"; do
        name=${entry%%|*}
        for want in "${only[@]}"; do
            [ "$name" = "$want" ] || [ "${name%%@*}" = "$want" ] && filtered+=("$entry")
        done
    done
    selected=("${filtered[@]}")
fi
if [ $list = 1 ]; then
    printf '%s\n' "${selected[@]}" | cut -d'|' -f1
    exit 0
fi

unset TMUX TMUX_PANE
private=$(mktemp -d "${TMPDIR:-/tmp}/cx-regressions.XXXXXX")
chmod 700 "$private"
export TMUX_TMPDIR="$private/tmux"
mkdir -p "$TMUX_TMPDIR" "$private/logs"
trap 'tmux -S "$TMUX_TMPDIR/tmux-$(id -u)/default" kill-server >/dev/null 2>&1; rm -rf "$private"' EXIT
if [ -n "${CX_PYTHONPATH:-}" ]; then
    export PYTHONPATH="$CX_PYTHONPATH${PYTHONPATH:+:$PYTHONPATH}"
fi

pass=0 fail=0 skip=0
failed=()
report() { printf '%-5s %s%s\n' "$1" "$2" "${3:+  ($3)}"; }

echo "== build"
if ! cargo build --locked >"$private/logs/build.log" 2>&1; then
    echo "FAIL  cargo build --locked"
    grep -E '^error' -A5 "$private/logs/build.log" | head -40
    exit 1
fi
BIN="$ROOT/target/debug/cx"

if [ $rust = 1 ]; then
    echo "== rust"
    if cargo fmt --check >"$private/logs/fmt.log" 2>&1; then
        report PASS "cargo fmt --check"; pass=$((pass + 1))
    else
        report FAIL "cargo fmt --check"; fail=$((fail + 1)); failed+=("cargo fmt --check")
    fi
    if [ $quick = 1 ]; then
        rust_cmds=("cargo test --locked --test regressions" "cargo test --locked --bin cx update::tests::relay_and_portable_tool_members_extract_but_other_members_are_refused")
    else
        rust_cmds=("cargo test --locked")
    fi
    for cmd in "${rust_cmds[@]}"; do
        log="$private/logs/rust.log"
        if $cmd >"$log" 2>&1; then
            counts=$(grep -E '^test result:' "$log" | awk '{p+=$4; f+=$6; i+=$8} END {print p" passed, "f" failed, "i" ignored"}')
            report PASS "$cmd" "$counts"; pass=$((pass + 1))
        else
            report FAIL "$cmd"; fail=$((fail + 1)); failed+=("$cmd")
            grep -E '^test .* FAILED|^---- |panicked' "$log" | head -20
        fi
    done
fi

have() {
    case "$1" in
    pyte) python3 -c 'import pyte' >/dev/null 2>&1 ;;
    fish) command -v fish >/dev/null 2>&1 ;;
    poppler) command -v pdftoppm >/dev/null 2>&1 && command -v pdfinfo >/dev/null 2>&1 ;;
    sshd) [ -x /usr/sbin/sshd ] || [ -x /usr/bin/sshd ] || command -v sshd >/dev/null 2>&1 ;;
    docker) docker info >/dev/null 2>&1 ;;
    node) command -v node >/dev/null 2>&1 ;;
    *) return 1 ;;
    esac
}

echo "== python fixtures (logs kept only on failure)"
command -v tmux >/dev/null 2>&1 || { echo "tmux is required for the fixtures"; exit 1; }
for entry in "${selected[@]}"; do
    IFS='|' read -r name needs args <<<"$entry"
    script=${name%%@*}
    variant=${name#*@}
    [ "$variant" = "$name" ] && variant=""
    missing=""
    for need in $needs; do have "$need" || missing="$missing $need"; done
    if [ -n "$missing" ]; then
        report SKIP "$name" "missing:$missing"; skip=$((skip + 1)); continue
    fi
    extra_env=()
    case "$variant" in
    sh) extra_env=(CX_TEST_SHELL=/bin/sh) ;;
    fish) extra_env=(CX_TEST_SHELL="$(command -v fish)") ;;
    esac
    argv=()
    for a in $args; do [ "$a" = BIN ] && argv+=("$BIN") || argv+=("$a"); done
    log="$private/logs/$name.log"
    start=$(date +%s)
    if env "${extra_env[@]}" timeout 600 python3 "tests/$script.py" "${argv[@]}" >"$log" 2>&1 </dev/null; then
        if [ "$script" = mesh_util ] && have node; then
            node tests/mesh_worker.mjs >>"$log" 2>&1 || { report FAIL "$name (mesh_worker.mjs)"; fail=$((fail + 1)); failed+=("$name"); continue; }
        fi
        report PASS "$name" "$(($(date +%s) - start))s"; pass=$((pass + 1))
    else
        code=$?
        report FAIL "$name" "exit $code, $(($(date +%s) - start))s"; fail=$((fail + 1)); failed+=("$name")
        # Fixtures print only synthetic data; still bound what reaches the terminal.
        tail -n 25 "$log" | cut -c1-240 | sed 's/^/      /'
    fi
done

echo "== summary: $pass passed, $fail failed, $skip skipped"
if [ $fail -gt 0 ]; then
    printf 'failed: %s\n' "${failed[*]}"
    exit 1
fi
