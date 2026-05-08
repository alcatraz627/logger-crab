#!/usr/bin/env bash
# check-s3.sh — verify the AWS credentials in .env work for the operations
# logger-crab performs against S3.
#
# Mirrors what S3ColdStore does at runtime:
#   1. head_bucket   (boot-time probe + every /health check)
#   2. put_object    (every rotation cycle)
#   3. get_object    (future cold-tier query API)
#
# Uses the AWS CLI (which produces real, non-collapsed error messages),
# so a failure here gives the actual AWS error code instead of the SDK's
# default "service error".
#
# Reads creds + bucket + region from ./.env (or ENV_FILE override).
# Run from the logger-crab repo root:
#   ./scripts/check-s3.sh

set -uo pipefail

# ─── Colors ────────────────────────────────────────────────────────────────
if [ -t 1 ]; then
  G=$'\033[0;32m'; R=$'\033[0;31m'; Y=$'\033[0;33m'
  D=$'\033[0;90m'; B=$'\033[1m'; X=$'\033[0m'
else
  G=''; R=''; Y=''; D=''; B=''; X=''
fi

pass() { printf "  ${G}✓${X} %s\n" "$1"; }
fail() { printf "  ${R}✗${X} %s\n" "$1"; }
warn() { printf "  ${Y}!${X} %s\n" "$1"; }
err()  { printf "${R}error:${X} %s\n" "$1" >&2; }

# ─── Pre-flight ────────────────────────────────────────────────────────────
if ! command -v aws >/dev/null 2>&1; then
  err "aws CLI not installed. Install with: brew install awscli"
  exit 2
fi

ENV_FILE="${ENV_FILE:-.env}"
if [ ! -f "$ENV_FILE" ]; then
  err "$ENV_FILE not found in $(pwd)"
  err "Create one with at minimum:"
  err "    AWS_ACCESS_KEY_ID=AKIA..."
  err "    AWS_SECRET_ACCESS_KEY=..."
  err "    AWS_REGION=us-east-1"
  err "    S3_LOGS_BUCKET=versable-logs"
  exit 2
fi
# shellcheck disable=SC1090
set -a; source "$ENV_FILE"; set +a

for v in AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_REGION S3_LOGS_BUCKET; do
  if [ -z "${!v:-}" ]; then
    err "$v not set in $ENV_FILE"
    exit 2
  fi
done

BUCKET="$S3_LOGS_BUCKET"
REGION="$AWS_REGION"
TEST_KEY="logger-crab-credential-check/$(date +%s).txt"
TEST_BODY="logger-crab credential check at $(date -u +%FT%TZ)"

printf "\n${B}═══ S3 credential check ═══${X}\n"
printf "  bucket   = %s\n" "$BUCKET"
printf "  region   = %s\n" "$REGION"
printf "  key id   = %s%s\n" "${AWS_ACCESS_KEY_ID:0:8}" "..."
printf "  test key = %s\n" "$TEST_KEY"
echo

FAILED=0

# ─── Test 1: head-bucket ───────────────────────────────────────────────────
printf "${B}Test 1 — head-bucket${X} (mirrors boot probe + /health)\n"
if out=$(aws s3api head-bucket --bucket "$BUCKET" --region "$REGION" 2>&1); then
  pass "head-bucket OK"
else
  fail "head-bucket FAILED"
  printf "${D}     %s${X}\n" "$out"
  FAILED=$((FAILED + 1))
  echo "    ${D}Common causes:${X}"
  echo "    ${D}- 404 Not Found      : bucket name typo OR wrong region${X}"
  echo "    ${D}- 403 Forbidden      : IAM user missing s3:ListBucket on bucket ARN${X}"
  echo "    ${D}- PermanentRedirect  : bucket exists in a different region than $REGION${X}"
  echo "    ${D}- InvalidAccessKeyId : access key doesn't exist${X}"
  echo "    ${D}- SignatureDoesNotMatch : secret key is wrong${X}"
  # If head-bucket fails everything else will too — bail.
  printf "\n${R}Aborting remaining tests.${X}\n"
  exit 1
fi
echo

# ─── Test 2: put-object ────────────────────────────────────────────────────
printf "${B}Test 2 — put-object${X} (mirrors rotation cron writes)\n"
TMP_BODY=$(mktemp)
printf "%s" "$TEST_BODY" > "$TMP_BODY"
if out=$(aws s3api put-object \
    --bucket "$BUCKET" \
    --region "$REGION" \
    --key "$TEST_KEY" \
    --body "$TMP_BODY" \
    --content-type "text/plain" \
    2>&1); then
  pass "put-object OK — wrote s3://$BUCKET/$TEST_KEY"
else
  fail "put-object FAILED"
  printf "${D}     %s${X}\n" "$out"
  FAILED=$((FAILED + 1))
  echo "    ${D}If head-bucket passed but put-object fails:${X}"
  echo "    ${D}- IAM user has s3:ListBucket but not s3:PutObject${X}"
  echo "    ${D}- Bucket policy denies writes from this principal${X}"
  echo "    ${D}- Object Lock or WORM policy blocking writes${X}"
fi
rm -f "$TMP_BODY"
echo

# ─── Test 3: get-object ────────────────────────────────────────────────────
printf "${B}Test 3 — get-object${X} (mirrors future cold-tier query)\n"
TMP_OUT=$(mktemp)
if out=$(aws s3api get-object \
    --bucket "$BUCKET" \
    --region "$REGION" \
    --key "$TEST_KEY" \
    "$TMP_OUT" \
    2>&1); then
  if [ "$(cat "$TMP_OUT")" = "$TEST_BODY" ]; then
    pass "get-object OK — round-trip body matches"
  else
    fail "get-object returned different body than we wrote"
    FAILED=$((FAILED + 1))
  fi
else
  fail "get-object FAILED"
  printf "${D}     %s${X}\n" "$out"
  FAILED=$((FAILED + 1))
fi
rm -f "$TMP_OUT"
echo

# ─── Test 4: cleanup (best-effort, IAM may not allow delete) ──────────────
printf "${B}Test 4 — delete-object${X} (cleanup; logger-crab itself does not need this)\n"
if out=$(aws s3api delete-object --bucket "$BUCKET" --region "$REGION" --key "$TEST_KEY" 2>&1); then
  pass "delete-object OK — test object cleaned up"
else
  warn "delete-object failed (expected — logger-crab's IAM policy doesn't grant s3:DeleteObject)"
  printf "${D}     %s${X}\n" "$out"
  printf "${D}     Manually clean up s3://%s/%s from the AWS Console if you want.${X}\n" "$BUCKET" "$TEST_KEY"
fi
echo

# ─── Summary ───────────────────────────────────────────────────────────────
if [ "$FAILED" -eq 0 ]; then
  printf "${G}${B}═══ All required ops work ═══${X}\n"
  echo
  echo "Your AWS credentials are correct for what logger-crab does."
  echo "If https://logger-crab.onrender.com/health still says cold.ok=false,"
  echo "the issue is on the Render side — likely one of:"
  echo "  - Different AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY in Render env"
  echo "  - AWS_REGION typo in Render env (must be \"$REGION\", exactly)"
  echo "  - S3_LOGS_BUCKET typo in Render env (must be \"$BUCKET\", exactly)"
  echo "  - COLD_STORE not set to \"s3\" in Render env"
  exit 0
else
  printf "${R}${B}═══ %d test(s) failed — credentials don't work ═══${X}\n" "$FAILED"
  echo
  echo "Fix the IAM policy / credentials before debugging Render."
  echo "Reference IAM policy: docs/STORAGE.md → 'IAM policy'"
  exit 1
fi
