#!/usr/bin/env bash
# smoke-ingest.sh — end-to-end smoke test for logger-crab /ingest endpoint.
#
# Loads tokens from `./.env` (run from the logger-crab repo root). The
# production URL is hardcoded; override by exporting LOGGER_CRAB_URL
# before running, or edit the URL block below.
#
# Validates:
#   1. Bad bearer is rejected (401)
#   2. Full-tier token is accepted (202)
#   3. Public-tier token is accepted (202)
#   4. Mixed-validity batch — accept good, reject bad with reason
#   5. Burst of 20 public-tier events
#   6. Dashboard reachable
#
# Tokens are read from .env in this priority order:
#   full   = LOGGER_CRAB_TOKEN_FULL   ?? LOGGER_CRAB_TOKEN
#   public = LOGGER_CRAB_TOKEN_PUBLIC ?? NEXT_PUBLIC_LOGGER_CRAB_TOKEN_PUBLIC
#
# Run with:
#   ./scripts/smoke-ingest.sh

set -uo pipefail

# ─── Hardcoded URL — points at production logger-crab ─────────────────────
LOGGER_CRAB_URL="${LOGGER_CRAB_URL:-https://logger-crab.onrender.com}"
LOGGER_CRAB_SERVICE="${LOGGER_CRAB_SERVICE:-local-smoke}"

# ─── Colors (only when TTY) ────────────────────────────────────────────────
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
err() { printf "${R}error:${X} %s\n" "$1" >&2; }

# ─── Source .env from cwd ──────────────────────────────────────────────────
ENV_FILE="${ENV_FILE:-.env}"
if [ ! -f "$ENV_FILE" ]; then
  err "no $ENV_FILE found in $(pwd)"
  err "create one with at minimum:"
  err "    LOGGER_CRAB_TOKEN=<full-tier token>"
  err "    NEXT_PUBLIC_LOGGER_CRAB_TOKEN_PUBLIC=<public-tier token>"
  err "or pass an alternate path: ENV_FILE=path/to/file ./scripts/smoke-ingest.sh"
  exit 1
fi
# shellcheck disable=SC1090
set -a; source "$ENV_FILE"; set +a

# ─── Resolve tokens (accept either naming convention) ──────────────────────
TF="${LOGGER_CRAB_TOKEN_FULL:-${LOGGER_CRAB_TOKEN:-}}"
TP="${LOGGER_CRAB_TOKEN_PUBLIC:-${NEXT_PUBLIC_LOGGER_CRAB_TOKEN_PUBLIC:-}}"

if [ -z "$TF" ]; then
  err "no full-tier token found in $ENV_FILE"
  err "expected LOGGER_CRAB_TOKEN_FULL or LOGGER_CRAB_TOKEN"
  exit 1
fi
if [ -z "$TP" ]; then
  err "no public-tier token found in $ENV_FILE"
  err "expected LOGGER_CRAB_TOKEN_PUBLIC or NEXT_PUBLIC_LOGGER_CRAB_TOKEN_PUBLIC"
  exit 1
fi

# ─── Identify which env var name was used (for the banner) ─────────────────
TF_SOURCE="$( [ -n "${LOGGER_CRAB_TOKEN_FULL:-}" ] && echo LOGGER_CRAB_TOKEN_FULL || echo LOGGER_CRAB_TOKEN )"
TP_SOURCE="$( [ -n "${LOGGER_CRAB_TOKEN_PUBLIC:-}" ] && echo LOGGER_CRAB_TOKEN_PUBLIC || echo NEXT_PUBLIC_LOGGER_CRAB_TOKEN_PUBLIC )"

URL="${LOGGER_CRAB_URL%/}"
SERVICE="$LOGGER_CRAB_SERVICE"
RID="smoke-$(date +%s)-$$"
TS="$(date -u +%FT%TZ)"

printf "\n${B}═══ logger-crab /ingest smoke ═══${X}\n"
info "URL:     $URL"
info "Service: $SERVICE"
info "Request: $RID"
info "Tokens:  full=\$$TF_SOURCE, public=\$$TP_SOURCE  (sourced from $ENV_FILE)"
echo

# ─── Test 0 — pre-flight: verify the new build is deployed ────────────────
printf "${B}Test 0${X} — pre-flight: verify new build is deployed (crab favicon route)\n"
code=$(curl -s -o /dev/null -w "%{http_code}" "$URL/assets/crab-logo.svg")
if [ "$code" = "200" ]; then
  pass "new build live (/assets/crab-logo.svg → 200)"
else
  fail "got $code on /assets/crab-logo.svg — old build still running?"
  err "Push the latest changes and wait for Render to redeploy, then re-run."
  echo
  printf "${R}${B}Aborted — pre-flight failed.${X}\n"
  exit 1
fi
echo

# ─── Test 1 — bad token → 401 ──────────────────────────────────────────────
printf "${B}Test 1${X} — bad bearer should return 401\n"
code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$URL/ingest" \
  -H "Authorization: Bearer definitely-not-a-real-token-123" \
  -H "Content-Type: application/json" \
  --data '{"events":[]}')
if [ "$code" = "401" ]; then
  pass "got 401 as expected"
else
  fail "expected 401, got $code"
  [ "$code" = "202" ] && info "server is in unauthenticated mode — check INGEST_TOKEN_* env vars in Render UI; settings modal will list malformed entries"
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
