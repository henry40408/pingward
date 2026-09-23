// Every script the UI runs, deferred. The CSP (`web::content_security_policy`)
// allows `script-src 'self'` with no 'unsafe-inline' or nonce, so templates
// carry no inline handlers: delegated handlers below key off `data-` attributes.
// Every page loads this file, so each block checks its target exists.

// --- swappable history sections (check pings/notifications, /admin audit) ---
window.pw = (function () {
  var pad = function (n) { return String(n).padStart(2, '0'); };

  // Localize timestamps under `root` (re-run after a fragment swap).
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

  // Fill each datetime-local from its data-utc, in local time, to the minute
  // (a blank seconds sub-field would empty .value).
  function fillDates(root) {
    root.querySelectorAll('input[type=datetime-local][data-utc]').forEach(function (el) {
      var v = el.getAttribute('data-utc'); if (!v) return;
      var d = new Date(v); if (isNaN(d.getTime())) return;
      el.value = d.getFullYear() + '-' + pad(d.getMonth() + 1) + '-' + pad(d.getDate()) +
        'T' + pad(d.getHours()) + ':' + pad(d.getMinutes());
    });
  }

  // datetime-local (local wall clock) -> UTC RFC 3339, '' if blank.
  function toUtc(val) {
    if (!val) return '';
    var d = new Date(val); if (isNaN(d.getTime())) return '';
    return d.toISOString().replace(/\.\d{3}Z$/, 'Z');
  }

  function initSection(section) { localize(section); bindToggles(section); fillDates(section); }

  // Pager/Clear links and the Apply button swap in the fragment. Returns the
  // loader, or null when the section is absent.
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

// A `data-href` row is a mouse convenience; the real route is the `<a>` on the
// name, which must stay (keyboard, middle-click, no-JS). Delegated so swapped-in
// rows work too.
document.addEventListener('click', function (e) {
  if (e.target.closest('a, button, input, select, textarea, label')) return;
  var row = e.target.closest('[data-href]');
  if (row) location = row.getAttribute('data-href');
});

// `data-confirm` forms ask first; `data-nosubmit` filter forms never submit
// (Apply fetches a fragment, and a stray Enter must not navigate).
document.addEventListener('submit', function (e) {
  var form = e.target;
  if (!form.getAttribute) return;
  if (form.hasAttribute('data-nosubmit')) { e.preventDefault(); return; }
  var message = form.getAttribute('data-confirm');
  if (message) {
    if (!confirm(message)) { e.preventDefault(); return; }
    // Without `?confirmed=1` the server renders the question as a page
    // (`ConfirmQuery`); a query param because some forms post no body.
    if (form.action.indexOf('confirmed=1') === -1) {
      form.action += (form.action.indexOf('?') === -1 ? '?' : '&') + 'confirmed=1';
    }
  }
  var action = form.getAttribute('data-reauth');
  if (action) { e.preventDefault(); askToConfirm(form, action); }
});

// --- admin re-authentication dialog ---
// `data-reauth` forms (rendered only while locked) ask for the password in
// place instead of bouncing to `/admin/unlock` and losing the form's input.
// The server re-checks regardless, and without JS the bounce still works.
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
      // Unlocked for a while; nothing else needs asking.
      var marked = document.querySelectorAll('[data-reauth]');
      for (var i = 0; i < marked.length; i++) marked[i].removeAttribute('data-reauth');
      reauthPending = null;
      reauthDialog.close();
      // `submit()` fires no submit event, so the handler above won't re-intercept.
      form.submit();
      return;
    }
    if (r.status === 403) { reauthError('That password is not correct.'); return; }
    if (r.status === 429) { reauthError('Too many attempts — try again later.'); return; }
    // Anything else: hand over to the page that can explain.
    location = '/admin/unlock';
  }).catch(function () { location = '/admin/unlock'; });
}

// --- theme toggle ---
(function () {
  var b = document.getElementById('pw-theme-toggle'); if (!b) return;
  var mq = matchMedia('(prefers-color-scheme: dark)');
  var order = ['light', 'dark', 'system'];

  // Stored preference; unset/unknown -> 'system'.
  function pref() {
    var p = null;
    try { p = localStorage.getItem('pw-theme'); } catch (e) {}
    return (p === 'light' || p === 'dark' || p === 'system') ? p : 'system';
  }

  // data-theme gets the effective light/dark; the button shows p itself.
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

// --- copy buttons ---
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

  // Opt-in live tail: an always-open EventSource per tab would eat the ~6
  // HTTP/1.1 connections per origin.
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
    // EventSource retries by itself; show "connecting" meanwhile, and turn
    // off once CLOSED (it will not come back).
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

// The check/channel forms' per-kind fields switch via `:has()` in `app.css`, not
// here: an inline `style.display` would outrank those rules.

// --- /admin: ticking heartbeat ages ---
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

// --- /admin: audit trail section ---
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
