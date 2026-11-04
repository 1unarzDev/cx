#!/bin/sh
# Leave the __MESH_NAME__ mesh (cx docs/mesh.md): curl -sL __MESH_URL__/leave|sh
# Deregisters this machine (its registration, service token and mesh name go on the server side) and restores
# what the join changed: the WARP MDM file, the service's state, the Tailscale rule, the ssh keys it added,
# and WARP itself when the join installed it.
set -eu
STATE=/var/lib/cx-mesh
DRY=${MESH_DRY_RUN:-}
OS=$(uname -s)
say() { printf '[mesh] %s\n' "$*" >&2; }
run() { if [ -n "$DRY" ]; then say "would run: $*"; else "$@"; fi; }
root() { if [ "$(id -u)" = 0 ]; then run "$@"; else run sudo "$@"; fi; }
rootq() { if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi; } # reads run even in a dry run
val() { printf '%s\n' "$1" | sed -n "s/^$2=//p" | head -1; }

DEV=$(rootq cat "$STATE/device" 2>/dev/null || true)
PRIOR=$(rootq cat "$STATE/prior" 2>/dev/null || true)
[ -n "$DEV$PRIOR" ] || { say "this machine is not on a mesh (no $STATE)"; exit 0; }
NAME=$(val "$DEV" NAME)
URL=$(val "$DEV" URL)
TOKEN=$(val "$DEV" TOKEN)
[ -n "$URL" ] || URL='__MESH_URL__'

if [ -n "$TOKEN" ] && [ -z "$DRY" ]; then
  R=$(curl -sS -X POST -H "authorization: Bearer $TOKEN" "$URL/v1/leave" 2>&1 || true)
  case $R in *'"ok":true'*) say "deregistered $NAME" ;; *) say "the server did not confirm ($R); ask the mesh owner to run: cx mesh revoke $NAME" ;; esac
fi

warp-cli --accept-tos disconnect >/dev/null 2>&1 || true
[ -n "$DRY" ] || warp-cli --accept-tos registration delete >/dev/null 2>&1 || true
if [ "$OS" = Darwin ]; then MDM='/Library/Managed Preferences/com.cloudflare.warp.plist'; else MDM=/var/lib/cloudflare-warp/mdm.xml; fi
root rm -f "$MDM"
if [ "$(val "$PRIOR" MDM_PRIOR)" = 1 ]; then root cp -p "$STATE/mdm.prior" "$MDM"; fi

if [ -e /etc/systemd/system/tailscaled.service.d/cx-mesh.conf ]; then
  P=$(sed -n 's/.*rule del priority \([0-9]*\).*/\1/p' /etc/systemd/system/tailscaled.service.d/cx-mesh.conf | tail -1)
  root rm -f /etc/systemd/system/tailscaled.service.d/cx-mesh.conf
  root rmdir /etc/systemd/system/tailscaled.service.d 2>/dev/null || true
  root systemctl daemon-reload
  [ -z "$P" ] || root ip rule del priority "$P" 2>/dev/null || true
fi
ip rule 2>/dev/null | grep -q 'fwmark 0x80000/0xff0000 lookup' && root ip rule del priority '__MESH_TAILSCALE_RULE__' 2>/dev/null || true

AK="$HOME/.ssh/authorized_keys"
if [ -f "$AK" ] && grep -q ' cx-mesh$' "$AK"; then
  run sh -c 'grep -v " cx-mesh$" "$1" >"$1.cx-mesh" || true; cat "$1.cx-mesh" >"$1"; rm -f "$1.cx-mesh"' sh "$AK"
fi

# The service as it was: stopped and disabled when it was before, and WARP removed when the join installed it.
if [ -d /run/systemd/system ] || [ "$OS" = Darwin ]; then
  if [ "$(val "$PRIOR" SVC_ENABLED)" != enabled ] && [ "$OS" != Darwin ]; then root systemctl disable --now warp-svc 2>/dev/null || true
  elif [ "$(val "$PRIOR" SVC_ACTIVE)" = active ] && [ "$OS" != Darwin ]; then root systemctl restart warp-svc
  fi
elif [ "$(val "$PRIOR" SVC_ACTIVE)" != active ]; then root pkill -x warp-svc || true
fi
if [ "$(val "$PRIOR" INSTALLED)" = 1 ]; then
  say "removing Cloudflare WARP (this join installed it)"
  if [ "$OS" = Darwin ]; then run brew uninstall --cask cloudflare-warp || true
  elif command -v apt-get >/dev/null 2>&1; then root apt-get remove -y -qq cloudflare-warp || true; root rm -f /etc/apt/sources.list.d/cloudflare-client.list
  elif command -v dnf >/dev/null 2>&1; then root dnf remove -y cloudflare-warp || true; root rm -f /etc/yum.repos.d/cloudflare-warp.repo
  elif command -v pacman >/dev/null 2>&1; then root pacman -Rns --noconfirm cloudflare-warp-bin || true
  fi
fi
root rm -rf "$STATE"
say "left the mesh${NAME:+ ($NAME)}; the previous network state is back"
