/* Applies the saved theme before first paint. Light is the default; the choice is stored in localStorage. */
(function () {
  var d = document.documentElement, t = 'light';
  try { var s = localStorage.getItem('keyreed-theme'); if (s === 'dark' || s === 'light') t = s; } catch (e) { /* storage blocked */ }
  d.setAttribute('data-theme', t);
  d.className += ' js';
  var m = document.querySelector('meta[name="theme-color"]');
  if (m) m.setAttribute('content', t === 'dark' ? '#060908' : '#f6f9f6');
})();
