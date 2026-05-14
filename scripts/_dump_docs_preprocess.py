#!/usr/bin/env python3
"""HTML preprocessor for ./dump-docs.sh.

Reads the rendered /docs HTML on stdin, swaps the three CSS-rendered
diagrams (arch / flow / cycle) for <pre> blocks containing ASCII art that
survives pandoc's HTML→markdown conversion intact. Writes the result to
stdout. Anything else passes through untouched.

The diagram divs in docs.rs are stable named classes followed immediately
by an <h2> (next section) or by a closing-paragraph element — we capture
from `<div class="arch-diagram">` to the byte before the next `<h2`.
"""

from __future__ import annotations

import os
import re
import sys

# Absolute origin to prepend onto server-relative hrefs. When the markdown
# is opened from disk, `<a href="/api">` would resolve to file:///api in
# Finder/Quick Look; rewriting to `https://logger-crab.../api` keeps every
# link clickable. Set by the calling shell script.
ORIGIN = os.environ.get("DUMP_DOCS_ORIGIN", "").rstrip("/")

ARCH_ASCII = """\
   +---------+  +---------+  +---------+  +---------+
   | Next.js |  | FastAPI |  | Worker  |  |  Cron   |
   | TS sink |  | Py sink |  | Node/Py |  |scheduled|
   +----+----+  +----+----+  +----+----+  +----+----+
        |            |            |            |
        +------------+------------+------------+
                            |
            POST /ingest * Bearer * X-Request-ID
                            v
                 +-------------------------+
                 |   axum * sqlx * maud    |
                 |  rust on render :10000  |
                 +------------+------------+
                              |  write hot * rotate cold
                              v
                +--------------+  +--------------+
                |  SQLite hot  |  |   S3 cold    |
                |  WAL * 48h   |  | NDJSON.gz    |
                +--------------+  +--------------+
"""

FLOW_ASCII = """\
  1. Browser -> Next.js middleware
     rid = headers['x-request-id'] ?? crypto.randomUUID()

  2. Next.js route -> FastAPI
     fetch(api, { headers: { 'x-request-id': rid } })

  3. FastAPI -> Redis enqueue
     job.request_id = request.state.request_id

  4. Worker pulls job
     rid = payload['request_id']     # same value, all 4 hops

  ==> filter dashboard by request_id=[rid]
      all 4 hops in chronological order, one screen
"""

CYCLE_ASCII = """\
  01. scan hot          02. group by hour     03. write S3 batch    04. delete hot
      ts < now - 48h  ->  (env, service, h)  ->  PUT *.ndjson.gz   ->  if all groups OK
"""

# Severity-scale rows live as `<div class="sev-row"><span class="sev-num">…
# <span class="sev-name">…<span class="sev-hint">…`. Pandoc strips the
# spans but inserts no separator, so all three values collapse into a
# single run ("1TRACEultra-verbose…"). Replace the whole wrapper with a
# pre-formatted table.
SEVERITY_ASCII = """\
   1   TRACE   ultra-verbose; follow-the-call-stack debug
   5   DEBUG   developer-only state info
   9   INFO    default - business events worth recording
  13   WARN    degraded state; not yet broken
  17   ERROR   user-visible failure
  21   FATAL   service crash / unrecoverable
"""


def replace_block(html: str, opening_class: str, ascii_block: str) -> str:
    """Replace a `<div class="{opening_class}"> ... </div>` block (and all
    its nested content) with `<pre>{ascii_block}</pre>`.

    Strategy: match from `<div class="{cls}">` non-greedily up to the byte
    before the next `<h2` or `<h3` heading — those mark the end of every
    diagram block in docs.rs. Using a lookahead boundary avoids the
    brace-matching problem inherent to regex-on-HTML.
    """
    pattern = re.compile(
        r'<div\s+class="' + re.escape(opening_class) + r'"[^>]*>'
        r'[\s\S]*?'
        r'(?=<h[23]\b)',
        flags=re.MULTILINE,
    )
    replacement = f"<pre>{ascii_block}</pre>\n\n"
    new_html, n = pattern.subn(replacement, html, count=1)
    if n == 0:
        sys.stderr.write(
            f"  [warn] preprocess: no match for div.{opening_class} "
            "(docs.rs structure may have changed)\n"
        )
    return new_html


def replace_link_cards(html: str) -> str:
    """Convert `<a class="link-card" href="X"><div class="title">T</div>
    <div class="sub">S</div></a>` into `- [T](X) - S` markdown. Pandoc
    otherwise drops the href and runs T+S together because the inner
    divs have no separator."""
    pattern = re.compile(
        r'<a\s+class="link-card"\s+href="([^"]+)"[^>]*>\s*'
        r'<div\s+class="title">([^<]+)</div>\s*'
        r'<div\s+class="sub">([^<]+)</div>\s*'
        r'</a>',
        flags=re.MULTILINE,
    )
    return pattern.sub(
        lambda m: f'<p><a href="{m.group(1)}">{m.group(2)}</a> - {m.group(3)}</p>',
        html,
    )


def absolutize_hrefs(html: str) -> str:
    """Rewrite `href="/path"` to `href="{ORIGIN}/path"` so the markdown
    stays clickable when opened from disk. In-page anchors (`href="#x"`)
    and already-absolute URLs (`http(s)://...`) pass through unchanged."""
    if not ORIGIN:
        return html
    # Match href="/..." but NOT href="//..." (protocol-relative) and NOT
    # href="/" alone — that one's the dashboard root, also rewrite.
    pattern = re.compile(r'href="(/[^"/][^"]*|/)"')
    return pattern.sub(lambda m: f'href="{ORIGIN}{m.group(1)}"', html)


def main() -> None:
    html = sys.stdin.read()
    html = replace_block(html, "arch-diagram", ARCH_ASCII)
    html = replace_block(html, "flow-diagram", FLOW_ASCII)
    html = replace_block(html, "cycle-diagram", CYCLE_ASCII)
    html = replace_block(html, "severity-scale", SEVERITY_ASCII)
    html = absolutize_hrefs(html)
    html = replace_link_cards(html)
    sys.stdout.write(html)


if __name__ == "__main__":
    main()
