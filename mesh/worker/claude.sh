#!/bin/sh
# Point Claude Code at a shared gateway (cx docs/claude.md): curl -sL __MESH_URL__/claude/<code>|sh
# Claims the lend once, merges ANTHROPIC_BASE_URL, ANTHROPIC_AUTH_TOKEN and (off the mesh) ANTHROPIC_CUSTOM_HEADERS
# into the env of Claude Code's settings.json (backed up first), then checks the gateway with one cheap request.
# Needs curl, and python3 or node to merge an existing settings.json. Secrets never reach argv or the terminal.
# Env: CLAUDE_CONFIG_DIR (default ~/.claude); CLAUDE_LEND_ROUTE=mesh|access forces a route.
set -eu
URL='__MESH_URL__'
CODE='__CLAUDE_CODE__'
MESH_URL='__CLAUDE_MESH_URL__'
MESH_ONLY='__CLAUDE_MESH_ONLY__'
DIR=${CLAUDE_CONFIG_DIR:-$HOME/.claude}
SETTINGS=$DIR/settings.json

say() { printf '[claude] %s\n' "$*" >&2; }
die() { say "error: $*"; exit 1; }
command -v curl >/dev/null 2>&1 || die "curl is required"
umask 077
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT INT TERM

# 1. the route: the mesh-only gateway when this machine reaches it, else the public one behind Access
ROUTE=access
if [ -n "$MESH_URL" ] && [ "${CLAUDE_LEND_ROUTE:-}" != access ]; then
  if [ "$(curl -s -m 4 -o /dev/null -w '%{http_code}' "$MESH_URL/health" 2>/dev/null || true)" = 200 ]; then ROUTE=mesh; fi
fi
[ "${CLAUDE_LEND_ROUTE:-}" = mesh ] && ROUTE=mesh
if [ "$ROUTE" = access ] && [ -n "$MESH_ONLY" ]; then
  die "this lend works only on the mesh, and $MESH_URL does not answer: join the mesh, then run this again (the code is still unused)"
fi
say "route: $ROUTE"

# 2. the claim: the secrets come back once, into a mode-600 file
printf '{"code":"%s","format":"lines"}' "$CODE" >"$T/req"
HTTP=$(curl -sS -m 30 -o "$T/lend" -w '%{http_code}' -H 'content-type: application/json' --data-binary @"$T/req" "$URL/v1/claude/claim") || die "could not reach $URL"
if [ "$HTTP" != 200 ]; then
  die "$(sed -n 's/.*"error" *: *"\([^"]*\)".*/\1/p' "$T/lend" | head -n1)"
fi
NAME= BASE= TOKEN=
HEADERS=
NL='
'
while IFS= read -r line; do
  case $line in
    name=*) NAME=${line#name=} ;;
    base_url=*) BASE=${line#base_url=} ;;
    token=*) TOKEN=${line#token=} ;;
    header=*) HEADERS=${HEADERS:+$HEADERS$NL}${line#header=} ;;
  esac
done <"$T/lend"
rm -f "$T/lend"
[ -n "$TOKEN" ] || die "the claim returned no key"
if [ "$ROUTE" = mesh ]; then BASE=$MESH_URL; HEADERS=; fi
[ -n "$BASE" ] || die "the lend has no gateway URL"

# 3. settings.json: merge into env, back up first
mkdir -p "$DIR"
if [ -f "$SETTINGS" ]; then
  B="$SETTINGS.bak-$(date +%Y%m%d-%H%M%S)"
  cp -p "$SETTINGS" "$B"
  say "backed up $SETTINGS to $B"
fi
MERGE_PY='import json,os,sys
p=sys.argv[1]
d=json.load(open(p)) if os.path.exists(p) and os.path.getsize(p) else {}
e=d.setdefault("env",{})
e["ANTHROPIC_BASE_URL"]=os.environ["L_BASE"]; e["ANTHROPIC_AUTH_TOKEN"]=os.environ["L_TOKEN"]
h=os.environ.get("L_HEADERS","")
if h: e["ANTHROPIC_CUSTOM_HEADERS"]=h
else: e.pop("ANTHROPIC_CUSTOM_HEADERS",None)
t=p+".tmp"
open(t,"w").write(json.dumps(d,indent=2)+"\n"); os.chmod(t,0o600); os.replace(t,p)'
MERGE_JS='const fs=require("fs"),p=process.argv[1],E=process.env;
let d={};try{const t=fs.readFileSync(p,"utf8");if(t.trim())d=JSON.parse(t)}catch(e){if(e.code!=="ENOENT")throw e}
const e=d.env=d.env||{};e.ANTHROPIC_BASE_URL=E.L_BASE;e.ANTHROPIC_AUTH_TOKEN=E.L_TOKEN;
if(E.L_HEADERS)e.ANTHROPIC_CUSTOM_HEADERS=E.L_HEADERS;else delete e.ANTHROPIC_CUSTOM_HEADERS;
fs.writeFileSync(p+".tmp",JSON.stringify(d,null,2)+"\n",{mode:0o600});fs.renameSync(p+".tmp",p)'
export L_BASE="$BASE" L_TOKEN="$TOKEN" L_HEADERS="$HEADERS"
if command -v python3 >/dev/null 2>&1 && python3 -c 'import json' 2>/dev/null; then
  python3 -c "$MERGE_PY" "$SETTINGS"
elif command -v node >/dev/null 2>&1; then
  node -e "$MERGE_JS" "$SETTINGS"
elif [ ! -s "$SETTINGS" ]; then # nothing to merge into: write it (the values hold no quotes or backslashes)
  H=$(printf '%s' "$HEADERS" | awk 'BEGIN{ORS=""} NR>1{print "\\n"} {print}')
  { printf '{\n  "env": {\n    "ANTHROPIC_BASE_URL": "%s",\n    "ANTHROPIC_AUTH_TOKEN": "%s"' "$BASE" "$TOKEN"
    [ -n "$HEADERS" ] && printf ',\n    "ANTHROPIC_CUSTOM_HEADERS": "%s"' "$H"
    printf '\n  }\n}\n'; } >"$SETTINGS"
else
  die "merging into $SETTINGS needs python3 or node"
fi
unset L_TOKEN L_HEADERS
say "wrote ANTHROPIC_BASE_URL ($BASE), ANTHROPIC_AUTH_TOKEN${HEADERS:+ and ANTHROPIC_CUSTOM_HEADERS} to $SETTINGS"
[ -z "${ANTHROPIC_API_KEY:-}" ] || say "note: ANTHROPIC_API_KEY is set in this shell and takes precedence; unset it for Claude Code"

# 4. one cheap request (token counting), headers from a mode-600 curl config, never argv
{ printf 'header = "authorization: Bearer %s"\n' "$TOKEN"
  printf '%s\n' "$HEADERS" | while IFS= read -r h; do [ -z "$h" ] || printf 'header = "%s"\n' "$h"; done; } >"$T/curl"
TOKEN=
HTTP=$(curl -sS -m 60 -K "$T/curl" -o "$T/out" -w '%{http_code}' -H 'content-type: application/json' -H 'anthropic-version: 2023-06-01' \
  --data '{"model":"claude-haiku-4-5","messages":[{"role":"user","content":"ok"}]}' "$BASE/v1/messages/count_tokens") || HTTP=000
if [ "$HTTP" = 200 ]; then
  say "${NAME:+$NAME: }ready (the gateway answered; start claude)"
else
  die "settings are written, but the gateway answered HTTP $HTTP; tell whoever sent the code"
fi
