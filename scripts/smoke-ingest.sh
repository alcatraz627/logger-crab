#!/usr/bin/env bash
# smoke-ingest.sh — end-to-end smoke test for logger-crab /ingest endpoint.
#
# Validates:
#   1. Bad bearer is rejected (401)
#   2. Full-tier token is accepted (202)
#   3. Public-tier token is accepted (202)
#   4. Batches with mixed-validity events accept the good ones, reject the bad
#   5. Burst of 20 public-tier events all land
#   6. Dashboard is reachable
#
# Required env vars (will prompt interactively if any are missing):
#   LOGGER_CRAB_URL              https://logger-crab.onrender.com
#   LOGGER_CRAB_TOKEN_FULL       full-tier token (server-side emitter)
#   LOGGER_CRAB_TOKEN_PUBLIC     public-tier token (browser-side emitter)
#   LOGGER_CRAB_SERVICE          optional, defaults to "local-smoke"
#
# Run with:
#   bash scripts/smoke-ingest.sh
#   (or chmod +x first, then: ./scripts/smoke-ingest.sh)

set -uo pipefail

# ─── Colors (only when TTY) ─────────────────────────────────────────────────
if [ -t 1 ]; then
  G=$'\033[0;32m'; R=$'\033[0;31m'; Y=$'\033[0;33m'
  D=$'\033[0;90m'; B=$'\033[1m'; X=$'\033[0m'
else
  G=''; R=''; Y=''; D=''; B=''; X=''
fi

PASSED=0
FAILED=0
pass() { printf "${G}✓${X} %s\n" "$1"; PASSED=$((PASSED + 1)); }
fail() { printf "${R}✗${X} %s\n" "$1"; FAILED=$((FAILED + 1)); }
info() { printf "${D}  %s${X}\n" "$1"; }

# ─── Env var checks + interactive prompts ──────────────────────────────────
prompt_if_missing() {
  local name="$1"
  local hint="$2"
  local current="${!name:-}"
  if [ -z "$current" ]; then
    printf "${Y}!${X} ${B}%s${X} is not set. ${D}%s${X}\n" "$name" "$hint"
    printf "  Enter value (or Ctrl-C to abort): "
    read -r value
    if [ -z "$value" ]; then
      printf "${R}Aborted: %s required.${X}\n" "$name"
      exit 1
    fi
    export "$name=$value"
  fi
}

prompt_if_missing LOGGER_CRAB_URL          "e.g. https://logger-crab.onrender.com"
prompt_if_missing LOGGER_CRAB_TOKEN_FULL   "full-tier token (mapped to a *-server consumer)"
prompt_if_missing LOGGER_CRAB_TOKEN_PUBLIC "public-tier token (mapped to a *-browser consumer)"
LOGGER_CRAB_SERVICE="${LOGGER_CRAB_SERVICE:-local-smoke}"

URL="${LOGGER_CRAB_URL%/}"
TF="$LOGGER_CRAB_TOKEN_FULL"
TP="$LOGGER_CRAB_TOKEN_PUBLIC"
SERVICE="$LOGGER_CRAB_SERVICE"
RID="smoke-$(date +%s)-$$"
TS="$(date -u +%FT%TZ)"

printf "\n${B}═══ logger-crab /ingest smoke ═══${X}\n"
info "URL:     $URL"
info "Service: $SERVICE"
info "Request: $RID"
echo

# ─── Test 1 — bad token → 401 ───────────────────────────────────────────────
printf "${B}Test 1${X} — bad bearer should return 401\n"
code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$URL/ingest" \
  -H "Authorization: Bearer definitely-not-a-real-token-123" \
  -H "Content-Type: application/json" \
  --data '{"events":[]}')
if [ "$code" = "401" ]; then
  pass "got 401 as expected"
else
  fail "expected 401, got $code"
fi
echo

# ─── Test 2 — full token, with _auth_consumer spoof attempt ────────────────
printf "${B}Test 2${X} — full-tier accept + server-stamp overwrites payload._auth_consumer\n"
resp=$(curl -s -X POST "$URL/ingest" \
  -H "Authorization: Bearer $TF" \
  -H "Content-Type: application/json" \
  --data @- <<EOF
{
  "resource": {"service": "$SERVICE", "env": "dev"},
  "scope": {"name": "smoke.sh", "version": "1"},
  "events": [{
    "event": "smoke.full",
    "severity_text": "info",
    "severity_number": 9,
    "ts": "$TS",
    "request_id": "$RID",
    "message": "full-token smoke",
    "payload": {"_auth_consumer": "hacker-trying-to-fake-this", "tier_test": "full"}
  }]
}
EOF
)
if printf '%s' "$resp" | grep -q '"accepted":1'; then
  pass "accepted=1 (verify _auth_consumer in dashboard — see below)"
else
  fail "expected accepted=1, got: $resp"
fi
echo

# ─── Test 3 — public token accept ──────────────────────────────────────────
printf "${B}Test 3${X} — public-tier accept\n"
resp=$(curl -s -X POST "$URL/ingest" \
  -H "Authorization: Bearer $TP" \
  -H "Content-Type: application/json" \
  --data @- <<EOF
{
  "resource": {"service": "$SERVICE", "env": "dev"},
  "events": [{
    "event": "smoke.public",
    "severity_text": "info",
    "severity_number": 9,
    "ts": "$TS",
    "request_id": "$RID",
    "message": "public-token smoke",
    "payload": {"tier_test": "public"}
  }]
}
EOF
)
if printf '%s' "$resp" | grep -q '"accepted":1'; then
  pass "accepted=1"
else
  fail "expected accepted=1, got: $resp"
fi
echo

# ─── Test 4 — batch with 1 valid + 1 invalid event ─────────────────────────
printf "${B}Test 4${X} — mixed batch: accepted=1, rejected=1 with reason 'missing request_id'\n"
resp=$(curl -s -X POST "$URL/ingest" \
  -H "Authorization: Bearer $TF" \
  -H "Content-Type: application/json" \
  --data @- <<EOF
{
  "resource": {"service": "$SERVICE", "env": "dev"},
  "events": [
    {"event": "smoke.batch.ok", "request_id": "$RID", "ts": "$TS"},
    {"event": "smoke.batch.bad-no-rid", "ts": "$TS"}
  ]
}
EOF
)
if printf '%s' "$resp" | grep -q '"accepted":1' && \
   printf '%s' "$resp" | grep -q 'missing request_id'; then
  pass "1 accepted, 1 rejected with expected reason"
else
  fail "expected accepted=1 + 'missing request_id' rejection, got: $resp"
fi
echo

# ─── Test 5 — burst 20 events on public token ──────────────────────────────
printf "${B}Test 5${X} — burst 20 public-tier events\n"
ok=0
for i in $(seq 1 20); do
  ts_i=$(date -u +%FT%TZ)
  burst_resp=$(curl -s -X POST "$URL/ingest" \
    -H "Authorization: Bearer $TP" \
    -H "Content-Type: application/json" \
    --data @- <<EOF
{"resource":{"service":"$SERVICE","env":"dev"},"events":[{"event":"smoke.burst.$i","severity_text":"info","severity_number":9,"ts":"$ts_i","request_id":"$RID-$i","message":"burst $i"}]}
EOF
)
  if printf '%s' "$burst_resp" | grep -q '"accepted":1'; then
    ok=$((ok + 1))
  fi
done
if [ "$ok" = "20" ]; then
  pass "20/20 events accepted"
else
  fail "$ok/20 events accepted"
fi
echo

# ─── Test 6 — dashboard reachability ───────────────────────────────────────
printf "${B}Test 6${X} — dashboard reachable\n"
code=$(curl -s -o /dev/null -w "%{http_code}" "$URL/?service=$SERVICE&env=dev")
if [ "$code" = "200" ] || [ "$code" = "401" ]; then
  pass "dashboard responds with $code"
  [ "$code" = "401" ] && info "dashboard is gated by DASHBOARD_TOKEN — set ?token=... or visit authenticated"
else
  fail "expected 200 or 401, got $code"
fi
echo

# ─── Summary ───────────────────────────────────────────────────────────────
TOTAL=$((PASSED + FAILED))
if [ "$FAILED" -eq 0 ]; then
  printf "${G}${B}═══ %d / %d passed ═══${X}\n" "$PASSED" "$TOTAL"
else
  printf "${R}${B}═══ %d / %d passed (%d failed) ═══${X}\n" "$PASSED" "$TOTAL" "$FAILED"
fi
echo

printf "${B}Verify visually in dashboard:${X}\n"
printf "  By request_id : %s/?request_id=%s\n" "$URL" "$RID"
printf "  By service    : %s/?service=%s&env=dev\n" "$URL" "$SERVICE"
printf "\n"
printf "  Click the ${B}smoke.full${X} row → expand the payload.\n"
printf "  ${G}PASS${X} if  payload._auth_consumer = a real consumer name (e.g. ${B}dev-aakarsh-server${X})\n"
printf "  ${R}FAIL${X} if  payload._auth_consumer = ${B}hacker-trying-to-fake-this${X}\n"
printf "                   ↑ that's a regression of the server-stamp guarantee.\n"
echo

[ "$FAILED" -eq 0 ] || exit 1
