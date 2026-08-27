// Every script the browser UI runs, in one deferred file.
//
// External rather than inline so the app can serve `script-src 'self'` with no
// `'unsafe-inline'` and no per-response nonce (see `web::security_headers`).
// For the same reason no template carries an `onclick=`/`onsubmit=` attribute —
// the delegated handlers below stand in for them, keyed off `data-` attributes.
//
// Loaded with `defer`, so the DOM is parsed before any of this runs. Each block
// is guarded by the presence of what it operates on, because every page loads
// the same file.

// --- shared helpers for the swappable history sections (check page pings +
//     notifications, /admin audit trail) ---
window.pw = (function () {
  var pad = function (n) { return String(n).padStart(2, '0'); };

  // Localize absolute timestamps within `root` to the viewer's zone, falling
  // back to the server-rendered UTC text. Takes a root so it can re-run on a
  // fragment after a partial swap.
  function localize(root) {
    root.querySelectorAll('.localtime[data-ts]').forEach(function (el) {
      var d = new Date(el.getAttribute('data-ts'));
      if (!isNaN(d.getTime())) {
        el.textContent = d.toLocaleString(undefined, { hour12: false });
        el.title = el.getAttribute('data-ts');
      }
    });
  }

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
  // Minute precision matches the inputs: a step=1 seconds sub-field left blank
  // would make .value empty on submit.
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
  // Returns the loader so a caller can re-fetch on its own, null when the
  // section is not on the page.
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

  function param(qs, key, val) { if (val) qs.push(key + '=' + encodeURIComponent(val)); }

  return { localize: localize, toUtc: toUtc, wireSection: wireSection, param: param };
})();

// --- delegated handlers, standing in for inline attributes ---

// A whole row acts as a link to `data-href`. Delegated from the document so it
// also covers rows inserted by a fragment swap; a click that landed on a real
// control is left alone.
//
// Mouse convenience only, not the row's link: the name inside each row is a
// real `<a>` to the same destination, and that is what carries keyboard access,
// the focus ring, the context menu, middle-click and the row working at all
// with JS off. Nothing here should grow back into the only way to reach a
// page.
document.addEventListener('click', function (e) {
  if (e.target.closest('a, button, input, select, textarea, label')) return;
  var row = e.target.closest('[data-href]');
  if (row) location = row.getAttribute('data-href');
});

// Destructive forms confirm first (`data-confirm`); the filter forms never
// submit at all (`data-nosubmit`) — their Apply button fetches a fragment
// instead, and a stray Enter in a filter field must not navigate away.
document.addEventListener('submit', function (e) {
  var form = e.target;
  if (!form.getAttribute) return;
  if (form.hasAttribute('data-nosubmit')) { e.preventDefault(); return; }
  var message = form.getAttribute('data-confirm');
  if (message) {
    if (!confirm(message)) { e.preventDefault(); return; }
    // Answered here, so tell the server as much: without the flag it refuses
    // and renders the same question as a page (`ConfirmQuery` in web.rs), which
    // is what a browser running no script gets. The flag goes in the query
    // string because several of these forms post no body at all.
    if (form.action.indexOf('confirmed=1') === -1) {
      form.action += (form.action.indexOf('?') === -1 ? '?' : '&') + 'confirmed=1';
    }
  }
  var action = form.getAttribute('data-reauth');
  if (action) { e.preventDefault(); askToConfirm(form, action); }
});

// --- admin re-authentication dialog ---
//
// The `/admin` controls that hand out access are single-button inline forms in
// a table row, so they cannot carry a password field of their own. Without this
// the server bounces them to `/admin/unlock` and whatever was typed into the
// form is gone; asking in place keeps the form intact.
//
// Progressive enhancement in both directions: the server refuses an unconfirmed
// action whatever the page did, and with JS off (or if this throws) the bounce
// still works. `data-reauth` is only rendered while locked. Built by delegation
// because the CSP (`script-src 'self'`, no nonce, no 'unsafe-inline') leaves no
// room for an inline handler.
var reauthDialog = null;
var reauthPending = null;

function buildReauthDialog() {
  var d = document.createElement('dialog');
  d.className = 'reauth';
  d.setAttribute('data-testid', 'reauth-dialog');
  d.innerHTML =
    '<form method="dialog" class="reauth-body">' +
    '<h2>Confirm it\'s you</h2>' +
    '<p class="crumb tight" data-testid="reauth-why">You\'re about to <strong data-testid="reauth-action"></strong>, ' +
    'which hands out access that keeps working after you sign out. ' +
    'This is your same password again, not a second factor.</p>' +
    '<p class="flash err" data-testid="reauth-error" hidden></p>' +
    '<div class="field"><label for="reauth-password">Password</label>' +
    '<input id="reauth-password" type="password" autocomplete="current-password" data-testid="reauth-input" required></div>' +
    '<div class="formactions">' +
    '<button class="btn primary" type="button" data-testid="reauth-submit">Confirm</button>' +
    '<button class="btn" type="button" data-testid="reauth-cancel">Cancel</button>' +
    '</div></form>';
  document.body.appendChild(d);
  d.querySelector('[data-testid="reauth-cancel"]').addEventListener('click', function () {
    reauthPending = null;
    d.close();
  });
  d.querySelector('[data-testid="reauth-submit"]').addEventListener('click', submitReauth);
  d.querySelector('[data-testid="reauth-input"]').addEventListener('keydown', function (e) {
    if (e.key === 'Enter') { e.preventDefault(); submitReauth(); }
  });
  return d;
}

function reauthError(text) {
  var p = reauthDialog.querySelector('[data-testid="reauth-error"]');
  p.textContent = text;
  p.hidden = !text;
}

function askToConfirm(form, action) {
  if (!reauthDialog) reauthDialog = buildReauthDialog();
  reauthPending = form;
  reauthDialog.querySelector('[data-testid="reauth-action"]').textContent = action;
  reauthError('');
  var input = reauthDialog.querySelector('[data-testid="reauth-input"]');
  input.value = '';
  reauthDialog.showModal();
  input.focus();
}

function submitReauth() {
  var form = reauthPending;
  if (!form) return;
  var input = reauthDialog.querySelector('[data-testid="reauth-input"]');
  var csrf = form.querySelector('input[name="_csrf"]');
  var body = new URLSearchParams();
  body.set('password', input.value);
  if (csrf) body.set('_csrf', csrf.value);
  fetch('/admin/unlock', {
    method: 'POST',
    headers: { 'X-Requested-With': 'fetch', 'Content-Type': 'application/x-www-form-urlencoded' },
    body: body.toString()
  }).then(function (r) {
    if (r.status === 204) {
      // Confirmed for a while, so nothing else needs asking either.
      var marked = document.querySelectorAll('[data-reauth]');
      for (var i = 0; i < marked.length; i++) marked[i].removeAttribute('data-reauth');
      reauthPending = null;
      reauthDialog.close();
      // `submit()`, not `requestSubmit()`: it fires no submit event, so the
      // handler above cannot intercept this one again.
      form.submit();
      return;
    }
    if (r.status === 403) { reauthError('That password is not correct.'); return; }
    if (r.status === 429) { reauthError('Too many attempts — try again later.'); return; }
    // Anything else (session gone, server trouble): hand over to the page that
    // can explain rather than leave the admin stuck in a dialog.
    location = '/admin/unlock';
  }).catch(function () { location = '/admin/unlock'; });
}

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

  // Resolve p to an effective light/dark for data-theme and reflect p itself in
  // the button glyph + labels.
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
  mq.addEventListener('change', function () { if (pref() === 'system') apply('system'); });
})();

pw.localize(document);

// --- copy buttons (API token on /account, ping URL on a check page) ---
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

  // Opt-in, not always-on: an EventSource held open by every check page would
  // spend one of the browser's ~6 HTTP/1.1 connections per origin, so a handful
  // of open tabs would stall the rest of the app.
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
    // EventSource retries transport errors itself, so there is no retry logic
    // here — but the button must stop claiming "open" while a retry is in
    // flight, and a CLOSED stream is never coming back (the check was deleted,
    // say), so drop the toggle to off.
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

// The check form's period/cron fields and the channel form's per-kind config
// blocks are switched by `:has()` rules in `app.css`, not here: an inline
// `style.display` would outrank those rules, so a script copy could only ever
// disagree with them (and with no script every branch showed at once).

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
// pings/notifications tables: same helper, same fragment contract.
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
