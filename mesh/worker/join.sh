#!/bin/sh
# Join the __MESH_NAME__ mesh (cx docs/mesh.md): curl -sL __MESH_URL__/<code>|sh
# Puts this machine on a Cloudflare One (WARP) mesh with one profile everywhere: internet over WARP (MASQUE),
# local networks untouched, the mesh reachable for ssh. Needs only curl and sudo; installs WARP when missing.
# Leave (restores what was there before): curl -sL __MESH_URL__/leave|sh
# Env: MESH_NAME=<name> picks the mesh name (default: this hostname); MESH_DRY_RUN=1 prints what it would do.
set -eu
URL='__MESH_URL__'
CODE='__MESH_CODE__'
TS_PRIO='__MESH_TAILSCALE_RULE__'
STATE=/var/lib/cx-mesh
DRY=${MESH_DRY_RUN:-}
OS=$(uname -s)

say() { printf '[mesh] %s\n' "$*" >&2; }
die() { say "error: $*"; exit 1; }
run() { if [ -n "$DRY" ]; then say "would run: $*"; else "$@"; fi; }
root() { if [ "$(id -u)" = 0 ]; then run "$@"; else run sudo "$@"; fi; }
rootq() { if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi; } # read-only checks run even in a dry run
# root_write FILE: stdin to FILE (mode 600), never through argv
root_write() { if [ -n "$DRY" ]; then say "would write $1"; cat >/dev/null; elif [ "$(id -u)" = 0 ]; then (umask 077; cat >"$1"); else sudo sh -c 'umask 077; cat >"$1"' sh "$1"; fi; }
field() { printf '%s' "$1" | sed -n "s/.*\"$2\" *: *\"\([^\"]*\)\".*/\1/p"; }
warp() { warp-cli --accept-tos "$@"; }

svc() { # restart | stop | enable | disable | is-enabled | is-active
  if [ "$OS" = Darwin ]; then
    L=system/com.cloudflare.1dot1dot1dot1.macos.warp.daemon
    case $1 in restart) root launchctl kickstart -k "$L" ;; is-*) echo enabled ;; *) : ;; esac
  elif [ -d /run/systemd/system ]; then
    case $1 in is-*) systemctl "$1" warp-svc 2>/dev/null || true ;; enable | disable) root systemctl "$1" --now warp-svc ;; *) root systemctl "$1" warp-svc ;; esac
  else # no init system (a container): the daemon directly
    case $1 in
      restart) root pkill -x warp-svc || true; [ -n "$DRY" ] || sleep 1; root sh -c 'nohup warp-svc >/var/log/warp-svc.log 2>&1 &' ;;
      stop | disable) root pkill -x warp-svc || true ;;
      is-active) pgrep -x warp-svc >/dev/null && echo active || echo inactive ;;
      is-enabled) echo unknown ;;
      *) : ;;
    esac
  fi
}

install_warp() {
  say "installing Cloudflare WARP"
  if [ "$OS" = Darwin ]; then
    if command -v brew >/dev/null 2>&1; then run brew install --cask cloudflare-warp; return; fi
    P=$(mktemp -d)/Cloudflare_WARP.pkg
    run curl -fsSL -o "$P" https://downloads.cloudflareclient.com/v1/download/macos/ga
    root installer -pkg "$P" -target /
  elif command -v apt-get >/dev/null 2>&1; then
    C=$(. /etc/os-release && echo "${VERSION_CODENAME:-}") # a subshell: os-release sets NAME too
    K=/usr/share/keyrings/cloudflare-warp-archive-keyring.gpg
    curl -fsSL https://pkg.cloudflareclient.com/pubkey.gpg | root gpg --yes --dearmor -o "$K"
    echo "deb [signed-by=$K] https://pkg.cloudflareclient.com/ ${C:-$(lsb_release -cs)} main" | root tee /etc/apt/sources.list.d/cloudflare-client.list >/dev/null
    root apt-get update -qq && root apt-get install -y -qq cloudflare-warp
  elif command -v dnf >/dev/null 2>&1; then
    curl -fsSL https://pkg.cloudflareclient.com/cloudflare-warp-ascii.repo | root tee /etc/yum.repos.d/cloudflare-warp.repo >/dev/null
    root dnf install -y cloudflare-warp
  elif command -v pacman >/dev/null 2>&1; then
    for h in paru yay; do
      if command -v "$h" >/dev/null 2>&1; then run "$h" -S --needed --noconfirm cloudflare-warp-bin; return; fi
    done
    # no AUR helper: build the AUR package with makepkg (as the ordinary user; it asks sudo to install)
    [ "$(id -u)" != 0 ] || die "as root there is no makepkg: re-run as your user, or install cloudflare-warp-bin from the AUR first"
    root pacman -S --needed --noconfirm git base-devel
    B=$(mktemp -d)
    run git clone -q https://aur.archlinux.org/cloudflare-warp-bin.git "$B/cloudflare-warp-bin"
    if [ -n "$DRY" ]; then say "would build and install cloudflare-warp-bin in $B"; else (cd "$B/cloudflare-warp-bin" && makepkg -si --noconfirm --needed); fi
  else
    die "no supported package manager; install Cloudflare WARP (https://pkg.cloudflareclient.com/), then re-run"
  fi
}

[ -n "$CODE" ] || die "this needs an invite code: ask the mesh owner for their command (curl -sL $URL/<code>|sh)"
case $OS in Linux | Darwin) ;; *) die "unsupported OS $OS (Windows: irm $URL/$CODE|iex in an administrator PowerShell)" ;; esac
command -v curl >/dev/null 2>&1 || die "curl is required"
[ "$(id -u)" = 0 ] || command -v sudo >/dev/null 2>&1 || die "run as root or install sudo"
if rootq test -e "$STATE/device"; then die "this machine is already on a mesh; leave first: curl -sL $URL/leave|sh"; fi

HOST=$(uname -n)
NAME=${MESH_NAME:-${HOST%%.*}}
INSTALLED=0
if ! command -v warp-cli >/dev/null 2>&1; then install_warp; INSTALLED=1; fi
[ -n "$DRY" ] || command -v warp-cli >/dev/null 2>&1 || die "warp-cli is still missing after the install"

# What leave restores: the service's state, any MDM file it replaces, and whether this run installed WARP.
if [ "$OS" = Darwin ]; then MDM='/Library/Managed Preferences/com.cloudflare.warp.plist'; else MDM=/var/lib/cloudflare-warp/mdm.xml; fi
root mkdir -p "$STATE" "$(dirname "$MDM")"
if ! rootq test -e "$STATE/prior"; then
  MDM_PRIOR=0
  if rootq test -e "$MDM"; then root cp -p "$MDM" "$STATE/mdm.prior"; MDM_PRIOR=1; fi
  printf 'INSTALLED=%s\nSVC_ENABLED=%s\nSVC_ACTIVE=%s\nMDM_PRIOR=%s\n' "$INSTALLED" "$(svc is-enabled)" "$(svc is-active)" "$MDM_PRIOR" | root_write "$STATE/prior"
fi

say "enrolling $NAME with $URL"
BODY=$(printf '{"code":"%s","name":"%s","host":"%s","os":"%s"}' "$CODE" "$NAME" "$HOST" "$OS")
if [ -n "$DRY" ]; then
  J='{"name":"'"$NAME"'","org":"dry-run","client_id":"id","client_secret":"secret","device_token":"tok","fqdn":"'"$NAME"'.mesh","ssh_user":""}'
else
  J=$(curl -sS -X POST -H 'content-type: application/json' --data "$BODY" "$URL/v1/enroll") || die "cannot reach $URL"
fi
ERR=$(field "$J" error)
[ -z "$ERR" ] || die "$ERR"
NAME=$(field "$J" name)
ORG=$(field "$J" org)
TOKEN=$(field "$J" device_token)
FQDN=$(field "$J" fqdn)
SSH_USER=$(field "$J" ssh_user)
printf 'NAME=%s\nURL=%s\nTOKEN=%s\nUSER=%s\n' "$NAME" "$URL" "$TOKEN" "$(id -un)" | root_write "$STATE/device"

# A previous registration (another org, a connector) gives way; the MDM file enrols with this device's service token.
warp disconnect >/dev/null 2>&1 || true
[ -n "$DRY" ] || warp registration delete >/dev/null 2>&1 || true
{
  [ "$OS" = Darwin ] && printf '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0">\n'
  printf '<dict>\n  <key>organization</key><string>%s</string>\n  <key>auth_client_id</key><string>%s</string>\n  <key>auth_client_secret</key><string>%s</string>\n  <key>onboarding</key><false/>\n</dict>\n' \
    "$ORG" "$(field "$J" client_id)" "$(field "$J" client_secret)"
  [ "$OS" = Darwin ] && printf '</plist>\n'
} | root_write "$MDM"
svc enable
svc restart

say "registering (up to 90 s)"
i=0
until [ -n "$DRY" ] || warp registration show >/dev/null 2>&1; do
  i=$((i + 1)); [ $i -lt 45 ] || die "WARP did not register; see warp-cli status"; sleep 2
done
run warp-cli --accept-tos connect >/dev/null
i=0
until [ -n "$DRY" ] || warp status 2>/dev/null | grep -q Connected; do
  i=$((i + 1)); [ $i -lt 30 ] || { say "WARP is registered but not connected yet: $(warp status 2>&1 | head -2 | tr '\n' ' ')"; break; }; sleep 2
done

# The mesh name points at this registration's addresses (the server reads them from Cloudflare, not from here).
IP4= IP6=
i=0
while [ -z "$DRY" ]; do
  R=$(curl -sS -X POST -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' --data "{\"host\":\"$HOST\"}" "$URL/v1/registered" || true)
  IP4=$(field "$R" ipv4); IP6=$(field "$R" ipv6)
  [ -z "$IP4$IP6" ] || break
  i=$((i + 1)); [ $i -lt 20 ] || { say "the mesh name is not set yet: $(field "$R" error)"; break; }; sleep 3
done

# Tailscale: WARP's Linux firewall passes only its tunnel and the excluded ranges, so Tailscale's own packets
# (fwmark 0x80000, sent past its routes) ride the WARP tunnel instead; LAN peers still fall through to main.
# Its firewall also drops CGNAT IPv4 that does not arrive on tailscale0, so ssh in over the mesh IPv6 address.
TS=0
if [ "$OS" = Linux ] && command -v tailscale >/dev/null 2>&1 && ip link show tailscale0 >/dev/null 2>&1; then
  TS=1
  T=$(ip rule 2>/dev/null | sed -n 's/.*fwmark 0x100cf lookup \([0-9]*\).*/\1/p' | head -1)
  T=${T:-65743}
  IP=$(command -v ip)
  RULE="$IP rule del priority $TS_PRIO 2>/dev/null; $IP rule add priority $TS_PRIO fwmark 0x80000/0xff0000 lookup $T"
  root sh -c "$RULE"
  if [ -d /run/systemd/system ]; then
    root mkdir -p /etc/systemd/system/tailscaled.service.d
    printf '[Service]\nExecStartPost=-/bin/sh -c "%s"\nExecStopPost=-%s rule del priority %s\n' "$RULE" "$IP" "$TS_PRIO" | root_write /etc/systemd/system/tailscaled.service.d/cx-mesh.conf
    root systemctl daemon-reload
  fi
  say "Tailscale found: its traffic now rides WARP (ip rule $TS_PRIO); reach this machine over the mesh IPv6 address"
fi

# The owner's ssh keys, marked so leave removes exactly these.
if [ -z "$DRY" ]; then
  AK="$HOME/.ssh/authorized_keys"
  mkdir -p "$HOME/.ssh" && chmod 700 "$HOME/.ssh"
  curl -fsS "$URL/v1/keys" | while IFS= read -r k; do
    [ -n "$k" ] || continue
    grep -qF "${k%% cx-mesh}" "$AK" 2>/dev/null || printf '%s cx-mesh\n' "$k" >>"$AK"
  done
  chmod 600 "$AK" 2>/dev/null || true
fi

LISTEN=1
if [ "$OS" = Linux ] && command -v ss >/dev/null 2>&1 && ! ss -ltn 2>/dev/null | grep -q ':22 '; then LISTEN=0; fi
U=${SSH_USER:-$(id -un)}
say "on the mesh as $NAME ($IP4 $IP6)"
[ $LISTEN = 1 ] || say "no ssh server is listening; start one (sudo systemctl enable --now sshd) to be reachable"
if [ $TS = 1 ] && [ -n "$IP6" ]; then echo "ssh $U@$IP6"; fi
echo "ssh $U@$FQDN"
say "leave: curl -sL $URL/leave|sh"
