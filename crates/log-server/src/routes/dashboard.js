(function () {
  // Theme toggle (persisted)
  var saved = localStorage.getItem("logger-crab-theme");
  if (saved === "light") document.body.classList.add("light");
  var toggle = document.getElementById("theme-toggle");
  if (toggle) {
    toggle.addEventListener("click", function () {
      document.body.classList.toggle("light");
      localStorage.setItem(
        "logger-crab-theme",
        document.body.classList.contains("light") ? "light" : "dark",
      );
    });
  }

  // Prefetch-on-hover. For same-origin GET links (filter chips, level
  // pills, page-size, pager, request_id), inject <link rel="prefetch">
  // when the user's cursor lingers 80ms. Dedupes per URL per page-load
  // so re-hovers don't re-fire. Idempotent GETs only — every chip on the
  // dashboard mutates the URL by toggling/adding query params, never POSTs.
  var prefetched = Object.create(null);
  var hoverTimer = null;
  function shouldPrefetch(a) {
    if (!a || !a.href) return false;
    if (a.target && a.target !== "" && a.target !== "_self") return false;
    if (a.hasAttribute("download")) return false;
    if (a.dataset && a.dataset.noPrefetch === "true") return false;
    if (a.origin !== window.location.origin) return false;
    if (a.href === window.location.href) return false;
    return true;
  }
  function prefetch(url) {
    if (prefetched[url]) return;
    prefetched[url] = true;
    var link = document.createElement("link");
    link.rel = "prefetch";
    link.href = url;
    link.as = "document";
    document.head.appendChild(link);
  }
  document.addEventListener("mouseover", function (ev) {
    var a = ev.target && ev.target.closest && ev.target.closest("a[href]");
    if (!shouldPrefetch(a)) return;
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = setTimeout(function () { prefetch(a.href); }, 80);
  });
  document.addEventListener("mouseout", function () {
    if (hoverTimer) { clearTimeout(hoverTimer); hoverTimer = null; }
  });

  // Client-side column sort
  var table = document.getElementById("events-table");
  if (!table) return;
  var tbody = table.querySelector("tbody");
  if (!tbody) return;

  var sortState = { key: "ts", dir: "desc" };

  function cmpValue(row, key) {
    var v = row.getAttribute("data-" + key);
    if (key === "ts" || key === "sev") return parseInt(v, 10) || 0;
    return (v || "").toLowerCase();
  }

  function applySort() {
    var rows = Array.prototype.slice.call(tbody.querySelectorAll("tr"));
    rows.sort(function (a, b) {
      var va = cmpValue(a, sortState.key);
      var vb = cmpValue(b, sortState.key);
      if (va < vb) return sortState.dir === "asc" ? -1 : 1;
      if (va > vb) return sortState.dir === "asc" ? 1 : -1;
      return 0;
    });
    // Re-append in order, re-stagger animation so sort feels live
    rows.forEach(function (r, i) {
      r.style.animation = "none";
      tbody.appendChild(r);
      // Force reflow so the re-animation actually restarts
      void r.offsetWidth;
      r.style.animation = "";
      r.style.animationDelay = Math.min(i, 15) * 15 + "ms";
    });

    // Visual indicator + aria-sort on headers
    var heads = table.querySelectorAll("th.sortable");
    heads.forEach(function (h) {
      h.classList.remove("sort-asc", "sort-desc");
      if (h.getAttribute("data-sort") === sortState.key) {
        h.classList.add(sortState.dir === "asc" ? "sort-asc" : "sort-desc");
        h.setAttribute(
          "aria-sort",
          sortState.dir === "asc" ? "ascending" : "descending",
        );
      } else {
        h.setAttribute("aria-sort", "none");
      }
    });
  }

  function activateSort(h) {
    var key = h.getAttribute("data-sort");
    if (sortState.key === key) {
      sortState.dir = sortState.dir === "asc" ? "desc" : "asc";
    } else {
      sortState.key = key;
      sortState.dir = key === "ts" || key === "sev" ? "desc" : "asc";
    }
    applySort();
  }
  table.querySelectorAll("th.sortable").forEach(function (h) {
    h.addEventListener("click", function () {
      activateSort(h);
    });
    h.addEventListener("keydown", function (ev) {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        activateSort(h);
      }
    });
  });

  // Initial indicator for default sort
  var defaultTh = table.querySelector('th.sortable[data-sort="ts"]');
  if (defaultTh) defaultTh.classList.add("sort-desc");

  // "/" keyboard shortcut — focus the full-text search input
  document.addEventListener("keydown", function (ev) {
    if (ev.key !== "/") return;
    var tag = (ev.target && ev.target.tagName) || "";
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
    var search = document.querySelector('form.filters input[name="q"]');
    if (search) {
      ev.preventDefault();
      search.focus();
      search.select();
    }
  });

  // Clicking a row (outside of links/details summary) toggles its details
  tbody.querySelectorAll("tr.evrow").forEach(function (row) {
    row.addEventListener("click", function (ev) {
      var t = ev.target;
      while (t && t !== row) {
        var tn = (t.tagName || "").toLowerCase();
        if (tn === "a" || tn === "summary" || tn === "input" || tn === "button")
          return;
        t = t.parentNode;
      }
      var det = row.querySelector("td.msg details");
      if (det) det.open = !det.open;
    });
  });
})();

// Filter form: drop empty inputs from the GET submission so the URL stays
// clean (no `request_id=&service=&env=dev&event_prefix=&q=` spam from
// blank fields). We disable empty inputs right before submit; browsers
// omit disabled fields from form data, then re-enable them so subsequent
// edits work normally. Cookies/state survive the round-trip.
(function () {
  var form = document.querySelector('form.filters');
  if (!form) return;
  form.addEventListener('submit', function () {
    var disabled = [];
    Array.from(form.elements).forEach(function (el) {
      if ((el.tagName === 'INPUT' || el.tagName === 'SELECT' || el.tagName === 'TEXTAREA')
          && !el.disabled
          && (el.value == null || el.value === '')) {
        el.disabled = true;
        disabled.push(el);
      }
    });
    // Re-enable on the next tick so the form is back to normal after submit.
    setTimeout(function () {
      disabled.forEach(function (el) { el.disabled = false; });
    }, 0);
  });
})();

// Timezone toggle — switches all timestamp cells between UTC and Local.
// Server renders ISO into [data-iso]; JS reformats based on toggle state
// and persists the choice to localStorage.
(function () {
  var TZ_KEY = 'logger-crab-tz';

  function currentTz() {
    return localStorage.getItem(TZ_KEY) || 'utc';
  }

  function fmtAbs(iso, tz) {
    var d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    var opts = {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hour12: false,
      timeZone: tz === 'local' ? undefined : 'UTC',
    };
    return new Intl.DateTimeFormat(undefined, opts).format(d);
  }

  function fmtRel(iso) {
    var d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    var diff = Math.round((Date.now() - d.getTime()) / 1000);
    if (diff < 5) return 'just now';
    if (diff < 60) return diff + 's ago';
    if (diff < 3600) return Math.round(diff / 60) + 'm ago';
    if (diff < 86400) return Math.round(diff / 3600) + 'h ago';
    return Math.round(diff / 86400) + 'd ago';
  }

  function applyTz() {
    var tz = currentTz();
    document.querySelectorAll('.ts-abs[data-iso]').forEach(function (el) {
      el.textContent = fmtAbs(el.getAttribute('data-iso'), tz);
      el.title = el.getAttribute('data-iso') + ' (' + tz + ')';
    });
    document.querySelectorAll('.ts-rel[data-iso]').forEach(function (el) {
      el.textContent = fmtRel(el.getAttribute('data-iso'));
      el.title = el.getAttribute('data-iso');
    });
    document.querySelectorAll('.tz-toggle-opt').forEach(function (btn) {
      var on = btn.getAttribute('data-tz') === tz;
      btn.classList.toggle('is-active', on);
      btn.setAttribute('aria-checked', on ? 'true' : 'false');
    });
  }

  document.addEventListener('click', function (e) {
    var btn = e.target.closest('.tz-toggle-opt');
    if (!btn) return;
    e.preventDefault();
    localStorage.setItem(TZ_KEY, btn.getAttribute('data-tz'));
    applyTz();
  });

  // Initial render — server emits UTC by default; this swaps to local
  // immediately on load if the user's preference is Local.
  applyTz();
  // Re-tick "Xm ago" every 30s so the relative times stay fresh.
  setInterval(function () {
    var tz = currentTz();
    document.querySelectorAll('.ts-rel[data-iso]').forEach(function (el) {
      el.textContent = fmtRel(el.getAttribute('data-iso'));
    });
  }, 30000);
})();

// Sticky-header offset auto-detection. Measures the nav's actual height
// and exposes it via CSS variable --nav-h so the sticky thead always sits
// flush below the nav, even if nav padding changes in the future.
(function () {
  function updateNavOffset() {
    var nav = document.querySelector('nav.lc-nav');
    if (!nav) return;
    document.documentElement.style.setProperty('--nav-h', nav.offsetHeight + 'px');
  }
  updateNavOffset();
  window.addEventListener('resize', updateNavOffset);
  // Re-measure once fonts load; web font metrics can shift the height by a few px.
  if (document.fonts && document.fonts.ready) {
    document.fonts.ready.then(updateNavOffset);
  }
})();

// Refresh button — reloads the current URL (preserves all filter state).
(function () {
  var btn = document.getElementById('refresh-btn');
  if (!btn) return;
  btn.addEventListener('click', function () {
    btn.classList.add('refreshing');
    location.reload();
  });
  // Keyboard shortcut: R (when not in an input)
  document.addEventListener('keydown', function (e) {
    if (e.key !== 'r' && e.key !== 'R') return;
    var t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    btn.click();
  });
})();

// Settings modal — open/close wiring.
(function () {
  var dialog = document.getElementById('settings-modal');
  var openBtn = document.getElementById('settings-open');
  var closeBtn = document.getElementById('settings-close');
  if (!dialog) return;
  if (openBtn) {
    openBtn.addEventListener('click', function () {
      if (typeof dialog.showModal === 'function') dialog.showModal();
      else dialog.setAttribute('open', '');
    });
  }
  if (closeBtn) {
    closeBtn.addEventListener('click', function () { dialog.close(); });
  }
  // Click on backdrop closes (native <dialog> does NOT do this by default).
  dialog.addEventListener('click', function (e) {
    if (e.target === dialog) dialog.close();
  });
})();

// Payload click isolation — clicking inside an expanded `.payload` (to
// select / copy text) must not bubble up to the parent `<details>` and
// collapse the row. Stop propagation at the `.payload` boundary on every
// mouse-related event that could re-enter the toggle path.
//
// We use the *capture* phase so this fires before any `<summary>` toggle
// handler higher in the tree. mouseup is also caught because text-drag
// selection ends with mouseup, and some browsers route that to the
// nearest interactive ancestor.
(function () {
  ['mousedown', 'mouseup', 'click', 'dblclick'].forEach(function (evt) {
    document.addEventListener(evt, function (e) {
      var target = e.target;
      if (target && target.closest && target.closest('.payload')) {
        e.stopPropagation();
      }
    }, true); // capture
  });
})();

// Copy buttons — request_id chips and payload JSON. Uses the Clipboard API
// with a graceful fallback to a hidden textarea + execCommand on older browsers.
(function () {
  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text);
    }
    var ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.focus(); ta.select();
    try { document.execCommand('copy'); } catch (e) {}
    document.body.removeChild(ta);
    return Promise.resolve();
  }
  function flashCopied(btn) {
    btn.classList.add('copied');
    var label = btn.getAttribute('aria-label');
    btn.setAttribute('aria-label', 'copied');
    setTimeout(function () {
      btn.classList.remove('copied');
      if (label) btn.setAttribute('aria-label', label);
    }, 900);
  }

  // Per-rid copy
  document.addEventListener('click', function (e) {
    var btn = e.target.closest('.rid-copy');
    if (!btn) return;
    e.preventDefault();
    e.stopPropagation();
    var text = btn.getAttribute('data-copy') || '';
    copyText(text).then(function () { flashCopied(btn); });
  });

  // Per-event payload copy
  document.addEventListener('click', function (e) {
    var btn = e.target.closest('.payload-copy');
    if (!btn) return;
    e.preventDefault();
    e.stopPropagation();
    var pre = btn.closest('.payload');
    var code = pre && pre.querySelector('.payload-json');
    if (!code) return;
    copyText(code.textContent || '').then(function () { flashCopied(btn); });
  });
})();

// Keyboard navigation — j/k cycle rows, Enter expands, Esc clears focus, ?
// shows shortcuts. Disabled while typing into inputs/textareas.
(function () {
  var rows = function () { return document.querySelectorAll('tr.evrow'); };
  var current = -1;

  function isTyping(e) {
    var t = e.target;
    if (!t) return false;
    if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable) return true;
    return false;
  }

  function setCurrent(idx) {
    var all = rows();
    if (!all.length) return;
    if (idx < 0) idx = 0;
    if (idx >= all.length) idx = all.length - 1;
    all.forEach(function (r) { r.classList.remove('row-focused'); });
    var r = all[idx];
    r.classList.add('row-focused');
    r.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    current = idx;
  }

  document.addEventListener('keydown', function (e) {
    if (isTyping(e)) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;

    if (e.key === 'j' || e.key === 'ArrowDown') {
      e.preventDefault();
      setCurrent(current + 1);
    } else if (e.key === 'k' || e.key === 'ArrowUp') {
      e.preventDefault();
      setCurrent(current - 1);
    } else if (e.key === 'Enter') {
      var all = rows();
      if (current >= 0 && current < all.length) {
        e.preventDefault();
        var det = all[current].querySelector('details');
        if (det) det.open = !det.open;
      }
    } else if (e.key === 'Escape') {
      document.activeElement && document.activeElement.blur && document.activeElement.blur();
    } else if (e.key === '?') {
      e.preventDefault();
      var help = document.getElementById('kbd-help-toast');
      if (help) {
        help.classList.toggle('visible');
        setTimeout(function () { help.classList.remove('visible'); }, 4500);
      }
    }
  });
})();
