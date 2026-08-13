// Resolve the applied theme before first paint (avoids a flash). The stored
// preference is 'light' | 'dark' | 'system'; anything else (incl. unset) means
// follow the OS. 'system' is re-resolved live by the listener in app.js.
//
// Its own file, loaded render-blocking from <head>, because that is the whole
// point: app.js is deferred and would run after the first paint has already
// happened in the wrong theme.
(function () {
  // Mark the document as scripted, before first paint and before anything
  // below can throw. Any CSS that hides content a click would reveal has to
  // hang off this class: without it the expandable rows stay open, so a
  // browser running no script can still read a failed job's captured output
  // instead of being left with a caret that does nothing. Set here rather
  // than in the deferred `app.js` because that runs after the first paint,
  // which would show every panel and then collapse them.
  document.documentElement.classList.add('js');
  try {
    var p = localStorage.getItem('pw-theme');
    var eff = (p === 'light' || p === 'dark')
      ? p
      : (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
    document.documentElement.setAttribute('data-theme', eff);
  } catch (e) {}
})();
