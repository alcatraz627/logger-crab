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
