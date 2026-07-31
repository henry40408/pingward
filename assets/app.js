// Every script the browser UI runs, in one deferred file.
//
// It is external rather than inline so the app can serve
// `script-src 'self'` with no `'unsafe-inline'` and no per-response nonce (see
// `web::security_headers`): a CSP that still allows inline script does not stop
// the injection it exists to stop, and nonces would mean threading a
// per-request value through every template struct. For the same reason there
// are no `onclick=`/`onsubmit=` attributes left in the templates — the
// delegated handlers below stand in for them, keyed off `data-` attributes.
//
// Loaded with `defer`, so the DOM is parsed before any of this runs and every
// block can look its elements up immediately. Each block is guarded by the
// presence of what it operates on, because every page loads the same file.

// --- shared helpers for the swappable history sections (check page pings +
//     notifications, /admin audit trail) ---
window.pw = (function () {
  var pad = function (n) { return String(n).padStart(2, '0'); };

  // Localize any absolute timestamps within `root` to the viewer's zone,
  // falling back to the server-rendered UTC text. Re-run per fragment after a
  // partial swap, which is why it takes a root.
  function localize(root) {
    root.querySelectorAll('.localtime[data-ts]').forEach(function (el) {
      var d = new Date(el.getAttribute('data-ts'));
      if (!isNaN(d.getTime())) {
        el.textContent = d.toLocaleString(undefined, { hour12: false });
        el.title = el.getAttribute('data-ts');
      }
    });
  }

  // Expand/collapse the detail row that follows a `tr.toggle`.
  function bindToggles(root) {
    root.querySelectorAll('tr.toggle').forEach(function (r) {
      r.addEventListener('click', function () {
        var n = r.nextElementSibling;
        if (n && n.classList.contains('exp')) {
          n.classList.toggle('open');
          r.querySelector('.caret').classList.toggle('open');
        }
      });
    });
  }

  // Fill each datetime-local from its data-utc (UTC instant) in local time.
  // Minute precision matches the inputs (no step=1 seconds sub-field, whose
  // being-left-blank would otherwise make .value empty on submit).
  function fillDates(root) {
    root.querySelectorAll('input[type=datetime-local][data-utc]').forEach(function (el) {
      var v = el.getAttribute('data-utc'); if (!v) return;
      var d = new Date(v); if (isNaN(d.getTime())) return;
      el.value = d.getFullYear() + '-' + pad(d.getMonth() + 1) + '-' + pad(d.getDate()) +
        'T' + pad(d.getHours()) + ':' + pad(d.getMinutes());
    });
  }

  // A datetime-local value (local wall clock) -> UTC RFC3339 (Z), '' if blank.
  function toUtc(val) {
    if (!val) return '';
    var d = new Date(val); if (isNaN(d.getTime())) return '';
    return d.toISOString().replace(/\.\d{3}Z$/, 'Z');
  }

  function initSection(section) { localize(section); bindToggles(section); fillDates(section); }

  // Wire a swappable history section: its pager/Clear links and Filter button
  // fetch the fragment endpoint and replace the section's contents in place.
  // Returns the loader so a caller can re-fetch on its own (the check page's
  // live tail does). Null when the section is not on the page.
  function wireSection(id, buildQuery) {
    var section = document.getElementById(id);
    if (!section) return null;
    var endpoint = section.getAttribute('data-endpoint');
    function load(url) {
      fetch(url, { headers: { 'X-Requested-With': 'fetch' } })
        .then(function (r) { return r.text(); })
        .then(function (html) { section.innerHTML = html; initSection(section); });
    }
    section.addEventListener('click', function (e) {
      var a = e.target.closest('a.btn');
      if (a && section.contains(a)) { e.preventDefault(); load(a.getAttribute('href')); return; }
      var b = e.target.closest('button[data-apply]');
      if (b && section.contains(b)) { e.preventDefault(); load(endpoint + buildQuery(section)); }
    });
    initSection(section);
    return load;
  }

  // Append `key=val` to a query-string accumulator, skipping blanks.
  function param(qs, key, val) { if (val) qs.push(key + '=' + encodeURIComponent(val)); }

  return { localize: localize, toUtc: toUtc, wireSection: wireSection, param: param };
})();

// --- delegated handlers, standing in for the removed inline attributes ---

// A whole row acts as a link to `data-href`. Delegated from the document so it
// also covers rows inserted by a fragment swap. A click that landed on a real
// control (the row's own "edit" link, a form button) is left alone — that is
// what the old `onclick="event.stopPropagation()"` on those links was for.
document.addEventListener('click', function (e) {
  if (e.target.closest('a, button, input, select, textarea, label')) return;
  var row = e.target.closest('[data-href]');
  if (row) location = row.getAttribute('data-href');
});

// Keyboard equivalent, for the same rows (they carry tabindex + role=link).
// Only when the row itself has focus: Enter inside a nested control belongs to
// that control.
document.addEventListener('keydown', function (e) {
  if (e.key !== 'Enter') return;
  var row = e.target.closest && e.target.closest('[data-href]');
  if (row && e.target === row) location = row.getAttribute('data-href');
});

// Destructive forms confirm first (`data-confirm`), and the filter forms never
// submit at all (`data-nosubmit`) — their Apply button fetches a fragment
// instead, and a stray Enter in a filter field must not navigate away.
document.addEventListener('submit', function (e) {
  var form = e.target;
  if (!form.getAttribute) return;
  if (form.hasAttribute('data-nosubmit')) { e.preventDefault(); return; }
  var message = form.getAttribute('data-confirm');
  if (message && !confirm(message)) e.preventDefault();
});

// --- theme toggle ---
(function () {
  var b = document.getElementById('pw-theme-toggle'); if (!b) return;
  var mq = matchMedia('(prefers-color-scheme: dark)');
  var order = ['light', 'dark', 'system'];

  // Stored preference, normalized: unset/unknown -> 'system' (follow OS).
  function pref() {
    var p = null;
    try { p = localStorage.getItem('pw-theme'); } catch (e) {}
    return (p === 'light' || p === 'dark' || p === 'system') ? p : 'system';
  }

  // Apply preference p: resolve to an effective light/dark for data-theme, and
  // reflect the current preference in the button glyph + labels.
  function apply(p) {
    var eff = (p === 'light' || p === 'dark') ? p : (mq.matches ? 'dark' : 'light');
    document.documentElement.setAttribute('data-theme', eff);
    b.textContent = (p === 'light' ? '☀' : p === 'dark' ? '☾' : '◐');
    b.setAttribute('title', 'Theme: ' + p + ' (click to change)');
    b.setAttribute('aria-label', 'Theme: ' + p + ' (click to change)');
    b.setAttribute('data-theme-pref', p);
  }

  apply(pref());
  b.addEventListener('click', function () {
    var next = order[(order.indexOf(pref()) + 1) % order.length];
    try { localStorage.setItem('pw-theme', next); } catch (e) {}
    apply(next);
  });
  // In 'system' mode, follow live OS colour-scheme changes.
  mq.addEventListener('change', function () { if (pref() === 'system') apply('system'); });
})();

// Render absolute timestamps (any `.localtime[data-ts]`) in the viewer's local
// time zone, falling back to the server-rendered UTC text when JS is off.
pw.localize(document);

// --- one-shot copy buttons (the API token on /account, the ping URL on a
//     check page) ---
document.querySelectorAll('.copy').forEach(function (btn) {
  btn.addEventListener('click', function () {
    var text = btn.getAttribute('data-copy');
    if (navigator.clipboard) navigator.clipboard.writeText(text);
  });
});

// --- check page: pings/notifications sections + the opt-in live tail ---
(function () {
  var loadPings = pw.wireSection('pings-section', function (s) {
    var qs = [];
    var kind = s.querySelector('[data-testid=pings-kind]');
    var from = s.querySelector('[data-testid=pings-from]');
    var to = s.querySelector('[data-testid=pings-to]');
    pw.param(qs, 'pk', kind && kind.value);
    pw.param(qs, 'pfrom', pw.toUtc(from && from.value));
    pw.param(qs, 'pto', pw.toUtc(to && to.value));
    return qs.length ? ('?' + qs.join('&')) : '';
  });
  pw.wireSection('notifs-section', function (s) {
    var qs = [];
    var ev = s.querySelector('[data-testid=notifs-event]');
    var st = s.querySelector('[data-testid=notifs-status]');
    var from = s.querySelector('[data-testid=notifs-from]');
    var to = s.querySelector('[data-testid=notifs-to]');
    pw.param(qs, 'ne', ev && ev.value);
    pw.param(qs, 'ns', st && st.value);
    pw.param(qs, 'nfrom', pw.toUtc(from && from.value));
    pw.param(qs, 'nto', pw.toUtc(to && to.value));
    return qs.length ? ('?' + qs.join('&')) : '';
  });

  // Live tail is opt-in, not always-on: an EventSource held open by every check
  // page would spend one of the browser's ~6 HTTP/1.1 connections per origin,
  // so a handful of open check tabs would stall the rest of the app.
  var liveBtn = document.getElementById('pings-live');
  var pingsSection = document.getElementById('pings-section');
  var pingsCard = document.getElementById('pings-card');
  if (!(liveBtn && pingsSection && loadPings && window.EventSource)) return;

  var liveSource = null;
  var liveTimer = null;
  function stopLive() {
    if (liveSource) { liveSource.close(); liveSource = null; }
    if (liveTimer) { clearTimeout(liveTimer); liveTimer = null; }
    if (pingsCard) pingsCard.classList.remove('live-on');
    liveBtn.setAttribute('aria-pressed', 'false');
    liveBtn.removeAttribute('data-live');
  }
  function startLive() {
    stopLive();
    liveBtn.setAttribute('aria-pressed', 'true');
    if (pingsCard) pingsCard.classList.add('live-on');
    liveBtn.setAttribute('data-live', 'connecting');
    liveSource = new EventSource(liveBtn.getAttribute('data-endpoint'));
    liveSource.onopen = function () { liveBtn.setAttribute('data-live', 'open'); };
    liveSource.onmessage = function () {
      if (liveTimer) clearTimeout(liveTimer);
      liveTimer = setTimeout(function () {
        loadPings(pingsSection.getAttribute('data-endpoint'));
      }, 500);
    };
    // EventSource retries transport errors on its own, so there is no manual
    // retry logic here — but the button must stop claiming "open" while that
    // retry is in flight. A CLOSED stream is never coming back (the endpoint
    // now 404s, say, because the check was deleted), so drop the toggle to off
    // rather than showing a live tail that isn't running.
    liveSource.onerror = function () {
      if (liveSource !== this) return; // stale handler from a replaced stream
      if (this.readyState === EventSource.CLOSED) stopLive();
      else liveBtn.setAttribute('data-live', 'connecting');
    };
  }
  liveBtn.addEventListener('click', function () {
    if (liveBtn.getAttribute('aria-pressed') === 'true') stopLive(); else startLive();
  });
  window.addEventListener('pagehide', stopLive);
})();

// --- forms whose visible fields depend on a <select> ---

// Check form: period vs cron.
(function () {
  var sel = document.getElementById('schedule_kind'); if (!sel) return;
  function sync() {
    document.querySelectorAll('.sched').forEach(function (d) {
      d.style.display = d.getAttribute('data-sched') === sel.value ? '' : 'none';
    });
  }
  sel.addEventListener('change', sync);
  sync();
})();

// Channel form: one config block per kind. The select is absent on the edit
// form — the kind is immutable there, so the single relevant block is already
// the only one rendered.
(function () {
  var sel = document.getElementById('kind'); if (!sel) return;
  function sync() {
    document.querySelectorAll('.cfg').forEach(function (d) {
      d.style.display = d.getAttribute('data-kind') === sel.value ? '' : 'none';
    });
  }
  sel.addEventListener('change', sync);
  sync();
})();

// --- /admin: ticking heartbeat ages + the audit trail section ---
(function () {
  if (!document.querySelector('.hb-ago[data-ago]')) return;
  function rel(ts) {
    var d = new Date(ts);
    if (isNaN(d.getTime())) return '';
    var s = Math.max(0, Math.round((Date.now() - d.getTime()) / 1000));
    if (s < 60) return s + 's ago';
    var m = Math.floor(s / 60);
    if (m < 60) return m + 'm ago';
    var h = Math.floor(m / 60);
    if (h < 24) return h + 'h ' + (m % 60) + 'm ago';
    var days = Math.floor(h / 24);
    return days + 'd ' + (h % 24) + 'h ago';
  }
  function tick() {
    document.querySelectorAll('.hb-ago[data-ago]').forEach(function (el) {
      el.textContent = rel(el.getAttribute('data-ago'));
    });
  }
  tick();
  setInterval(tick, 1000);
})();

// The audit trail is a swappable history section like the check page's
// pings/notifications tables — same helper, same fragment contract.
pw.wireSection('audit-section', function (s) {
  var qs = [];
  var actor = s.querySelector('[data-testid=audit-actor]');
  var action = s.querySelector('[data-testid=audit-action]');
  var from = s.querySelector('[data-testid=audit-from]');
  var to = s.querySelector('[data-testid=audit-to]');
  pw.param(qs, 'aactor', actor && actor.value);
  pw.param(qs, 'aaction', action && action.value);
  pw.param(qs, 'afrom', pw.toUtc(from && from.value));
  pw.param(qs, 'ato', pw.toUtc(to && to.value));
  return qs.length ? '?' + qs.join('&') : '';
});
