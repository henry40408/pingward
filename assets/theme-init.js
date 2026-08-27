// Resolve the applied theme before first paint (avoids a flash). The stored
// preference is 'light' | 'dark' | 'system'; anything else means follow the OS,
// and 'system' is re-resolved live by the listener in app.js. Render-blocking
// in its own file because app.js is deferred and runs after the first paint.
(function () {
  // Mark the document as scripted, before first paint and before anything
  // below can throw. CSS that hides content a click would reveal hangs off this
  // class, so a browser running no script keeps the expandable rows open. Not
  // in the deferred `app.js`, which runs after the first paint and would show
  // every panel and then collapse it.
  document.documentElement.classList.add('js');
  try {
    var p = localStorage.getItem('pw-theme');
    var eff = (p === 'light' || p === 'dark')
      ? p
      : (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
    document.documentElement.setAttribute('data-theme', eff);
  } catch (e) {}
})();
