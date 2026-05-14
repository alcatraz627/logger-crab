#!/usr/bin/env bash
# Render /docs to markdown and open it. Inspection-only — not used in prod.
#
# Usage:
#   ./scripts/dump-docs.sh                 # boot local server, fetch, convert, open
#   ./scripts/dump-docs.sh <URL> <TOKEN>   # fetch from an already-running server
#                                          # e.g. https://logger-crab.onrender.com $DASHBOARD_TOKEN
#
# Output: ./_docs.md in the repo root (override via OUT=/path).
# `_docs.md` is gitignored — overwrite-safe between runs.
set -euo pipefail

URL="${1:-}"
TOKEN="${2:-}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${OUT:-${REPO_ROOT}/_docs.md}"
PORT="${PORT:-18099}"

command -v pandoc >/dev/null || {
  echo "pandoc required — install with: brew install pandoc" >&2
  exit 1
}

if [[ -z "$URL" ]]; then
  TOKEN="${TOKEN:-dev}"
  URL="http://127.0.0.1:${PORT}"
  echo "→ booting local server on :${PORT} with DASHBOARD_TOKEN=${TOKEN}"

  DASHBOARD_TOKEN="$TOKEN" HOT_STORE=memory COLD_STORE=noop \
    DATABASE_URL="sqlite::memory:" PORT="$PORT" \
    cargo run -p log-server >/tmp/dump-docs.cargo.log 2>&1 &
  SERVER_PID=$!
  trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

  echo -n "→ waiting for ready"
  for _ in $(seq 1 60); do
    if curl -sf "${URL}/health" >/dev/null 2>&1; then echo " ✓"; break; fi
    echo -n "."; sleep 1
  done
fi

if [[ -z "$TOKEN" ]]; then
  echo "→ remote URL given but no token — pass TOKEN as 2nd arg" >&2
  exit 1
fi

echo "→ fetching ${URL}/docs"
RAW="$(curl -sSf -H "Authorization: Bearer ${TOKEN}" "${URL}/docs")"

# Pandoc cannot infer that <div class="arch-box"> is a horizontal row item —
# it flattens each div onto its own line, producing a vertical word soup.
# Replace the three CSS-rendered diagrams with <pre> ASCII equivalents so
# pandoc preserves them as fenced code blocks.
echo "→ preprocessing diagrams to ASCII"
# DUMP_DOCS_ORIGIN: rewrite server-relative hrefs to absolute URLs so the
# markdown stays clickable when opened from disk. Defaults to the URL we
# fetched from; override with ORIGIN=https://logger-crab.onrender.com to
# make links survive the local dev server being torn down.
PROCESSED="$(
  printf '%s' "$RAW" \
    | DUMP_DOCS_ORIGIN="${ORIGIN:-${URL}}" \
        python3 "$(dirname "$0")/_dump_docs_preprocess.py"
)"

printf '%s' "$PROCESSED" | pandoc -f html -t gfm --wrap=none -o "$OUT"

# Strip pandoc's empty image/anchor artifacts that survive the conversion.
sed -i '' -e '/^!\[\](/d' -e '/^\[\](#)/d' "$OUT" 2>/dev/null || true

echo "→ wrote $(wc -l < "$OUT" | tr -d ' ') lines to $OUT"
open "$OUT"
