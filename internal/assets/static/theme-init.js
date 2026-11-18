// Render-blocking, unlike the deferred app.js: sets the theme before first paint
// (no flash). Anything but 'light'/'dark' follows the OS.
(function () {
  // First, before anything can throw: CSS that hides click-to-reveal content
  // hangs off `.js`, so a scriptless browser keeps it visible.
  document.documentElement.classList.add('js');
  try {
    var p = localStorage.getItem('pw-theme');
    var eff = (p === 'light' || p === 'dark')
      ? p
      : (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
    document.documentElement.setAttribute('data-theme', eff);
  } catch (e) {}
})();
