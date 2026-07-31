// Resolve the applied theme before first paint (avoids a flash). The stored
// preference is 'light' | 'dark' | 'system'; anything else (incl. unset) means
// follow the OS. 'system' is re-resolved live by the listener in app.js.
//
// Its own file, loaded render-blocking from <head>, because that is the whole
// point: app.js is deferred and would run after the first paint has already
// happened in the wrong theme.
(function () {
  try {
    var p = localStorage.getItem('pw-theme');
    var eff = (p === 'light' || p === 'dark')
      ? p
      : (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
    document.documentElement.setAttribute('data-theme', eff);
  } catch (e) {}
})();
