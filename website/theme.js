/* Applies saved theme and motion flags before first paint. Light is the default; sound is off by default. */
(function () {
  var d = document.documentElement, t = 'light', snd = 'off';
  try {
    var s = localStorage.getItem('keyreed-theme');
    if (s === 'dark' || s === 'light') t = s;
    if (localStorage.getItem('keyreed-sound') === 'on') snd = 'on';
  } catch (e) { /* storage blocked */ }
  d.setAttribute('data-theme', t);
  d.setAttribute('data-sound', snd);
  d.classList.add('js');
  if (window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches) d.classList.add('rm');
  var m = document.querySelector('meta[name="theme-color"]');
  if (m) m.setAttribute('content', t === 'dark' ? '#070a09' : '#ffffff');
})();
