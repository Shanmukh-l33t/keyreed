/* Core UI module: navigation, mobile sheet, theme & sound toggles, FAQ, copy buttons,
   live GitHub stars & release hydration, and in-page TOC scroll-spy. */

import { initMotion, isReducedMotion, setScrollLocked, sound } from './motion.js';

const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
const REPO = 'Shanmukh-l33t/keyreed';

initMotion();

/* ---------- Theme toggle (light is the default) ---------- */
const root = document.documentElement;
const themeBtn = $('#theme-btn');
const metaTheme = $('meta[name="theme-color"]');

function syncThemeUi(theme) {
  root.setAttribute('data-theme', theme);
  if (metaTheme) metaTheme.setAttribute('content', theme === 'dark' ? '#070a09' : '#ffffff');
  if (!themeBtn) return;
  const next = theme === 'dark' ? 'light' : 'dark';
  themeBtn.setAttribute('aria-label', `Switch to ${next} theme`);
  themeBtn.title = `Switch to ${next} theme`;
}

syncThemeUi(root.getAttribute('data-theme') === 'dark' ? 'dark' : 'light');

if (themeBtn) {
  themeBtn.addEventListener('click', () => {
    const next = root.getAttribute('data-theme') === 'dark' ? 'light' : 'dark';
    if (!isReducedMotion()) {
      root.classList.add('theming');
      setTimeout(() => root.classList.remove('theming'), 360);
    }
    syncThemeUi(next);
    try { localStorage.setItem('keyreed-theme', next); } catch { /* storage blocked */ }
  });
}

/* ---------- Sound toggle (off by default, persisted in localStorage) ---------- */
const soundBtns = $$('[data-sound-toggle], #sound-btn');

function syncSoundUi() {
  const on = sound.isEnabled();
  soundBtns.forEach((btn) => {
    btn.setAttribute('aria-pressed', String(on));
    const label = on ? 'Mute sound effects' : 'Turn sound effects on';
    btn.setAttribute('aria-label', label);
    btn.title = label;
  });
}

syncSoundUi();

soundBtns.forEach((btn) => {
  btn.addEventListener('click', () => {
    sound.setEnabled(!sound.isEnabled());
    syncSoundUi();
  });
});

/* ---------- Sticky glassy header & full-screen mobile sheet ---------- */
const nav = $('#nav');
const menuBtn = $('#menu-btn');
const mobileNav = $('#mobile-nav');

function syncNavScroll() {
  if (!nav) return;
  const menuOpen = mobileNav && !mobileNav.hidden;
  nav.classList.toggle('scrolled', window.scrollY > 12 || Boolean(menuOpen));
}

window.addEventListener('scroll', syncNavScroll, { passive: true });
syncNavScroll();

if (menuBtn && mobileNav) {
  const links = $$('a, button', mobileNav);
  links.forEach((el, idx) => el.style.setProperty('--i', String(idx)));

  const setMenu = (open) => {
    menuBtn.setAttribute('aria-expanded', String(open));
    menuBtn.setAttribute('aria-label', open ? 'Close menu' : 'Open menu');
    const useEl = menuBtn.querySelector('use');
    if (useEl) useEl.setAttribute('href', open ? '#i-close' : '#i-menu');
    mobileNav.hidden = !open;
    if (open) {
      requestAnimationFrame(() => mobileNav.classList.add('open'));
    } else {
      mobileNav.classList.remove('open');
    }
    setScrollLocked(open);
    syncNavScroll();
  };

  menuBtn.addEventListener('click', () => {
    setMenu(menuBtn.getAttribute('aria-expanded') !== 'true');
  });

  mobileNav.addEventListener('click', (e) => {
    if (e.target.closest('a')) setMenu(false);
  });

  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !mobileNav.hidden) {
      setMenu(false);
      menuBtn.focus();
    }
  });

  window.matchMedia('(min-width: 981px)').addEventListener('change', (e) => {
    if (e.matches && !mobileNav.hidden) setMenu(false);
  });
}

/* ---------- Scroll-spy for primary nav & /details sticky TOC ---------- */
function attachSpy(selector, attr, margin) {
  const links = $$(selector);
  if (!links.length || !('IntersectionObserver' in window)) return;
  const map = new Map(links.map((a) => [a.getAttribute(attr), a]));
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      map.forEach((lnk, id) => lnk.classList.toggle('active', id === e.target.id));
    }
  }, { rootMargin: margin });
  map.forEach((_, id) => {
    const sec = id && document.getElementById(id);
    if (sec) io.observe(sec);
  });
}

attachSpy('[data-spy]', 'data-spy', '-35% 0px -55% 0px');
attachSpy('[data-toc]', 'data-toc', '-20% 0px -68% 0px');

/* ---------- Smooth FAQ accordion ---------- */
$$('.qa button').forEach((btn) => {
  btn.addEventListener('click', () => {
    const qa = btn.closest('.qa');
    if (!qa) return;
    const next = btn.getAttribute('aria-expanded') !== 'true';
    btn.setAttribute('aria-expanded', String(next));
    qa.classList.toggle('open', next);
  });
});

/* ---------- Copy-to-clipboard buttons ---------- */
$$('[data-copy-from]').forEach((btn) => {
  const initialHtml = btn.innerHTML;
  const initialLabel = btn.getAttribute('aria-label') || 'Copy';
  let resetTimer = 0;
  btn.addEventListener('click', async () => {
    const src = document.getElementById(btn.dataset.copyFrom);
    if (!src) return;
    const text = src.textContent.trim();
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('readonly', '');
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      try { document.execCommand('copy'); } catch { /* ignore */ }
      ta.remove();
    }
    sound.keyUp();
    clearTimeout(resetTimer);
    btn.classList.add('done');
    btn.setAttribute('aria-label', 'Copied');
    btn.innerHTML = '<svg class="ic" aria-hidden="true"><use href="#i-check"/></svg>';
    resetTimer = window.setTimeout(() => {
      btn.classList.remove('done');
      btn.setAttribute('aria-label', initialLabel);
      btn.innerHTML = initialHtml;
    }, 1600);
  });
});

/* ---------- Finale giant keycap tactile press ---------- */
const finaleKey = $('#finale-download');
if (finaleKey) {
  finaleKey.addEventListener('pointerdown', () => {
    finaleKey.classList.add('down');
    sound.keyDown();
  });
  const release = () => finaleKey.classList.remove('down');
  finaleKey.addEventListener('pointerup', release);
  finaleKey.addEventListener('pointerleave', release);
  finaleKey.addEventListener('pointercancel', release);
}

/* ---------- Live GitHub stars & latest release hydration (cached, graceful fallback) ---------- */
const fmtCount = (n) => (n >= 1000 ? `${(n / 1000).toFixed(n >= 10000 ? 0 : 1)}k` : String(n));
const fmtMb = (bytes) => `${(bytes / 1e6).toFixed(1)} MB`;

async function fetchCachedJson(cacheKey, url) {
  try {
    const raw = sessionStorage.getItem(cacheKey);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed && Date.now() - parsed.t < 3600_000) return parsed.d;
    }
  } catch { /* ignore */ }
  const res = await fetch(url, { headers: { Accept: 'application/vnd.github+json' } });
  if (!res.ok) return null;
  const data = await res.json();
  try { sessionStorage.setItem(cacheKey, JSON.stringify({ t: Date.now(), d: data })); } catch { /* ignore */ }
  return data;
}

async function hydrateGitHub() {
  const repo = await fetchCachedJson('kr-gh-repo-v3', `https://api.github.com/repos/${REPO}`).catch(() => null);
  if (repo && typeof repo.stargazers_count === 'number' && repo.stargazers_count > 0) {
    const formatted = fmtCount(repo.stargazers_count);
    $$('[data-gh-stars]').forEach((el) => {
      el.textContent = formatted;
      el.hidden = false;
    });
  }

  const rel = await fetchCachedJson('kr-gh-rel-v3', `https://api.github.com/repos/${REPO}/releases/latest`).catch(() => null);
  if (!rel || !rel.tag_name) return;
  const ver = String(rel.tag_name).replace(/^v/, '');
  if (!/^\d+\.\d+\.\d+/.test(ver)) return;

  $$('[data-bind="version"]').forEach((el) => { el.textContent = ver; });
  const assets = Array.isArray(rel.assets) ? rel.assets : [];
  const setup = assets.find((a) => a.name === 'KeyReed-Setup-x64.exe');
  const portable = assets.find((a) => a.name && a.name.endsWith('portable.zip'));

  if (setup) {
    $$('[data-bind="installer-size"]').forEach((el) => { el.textContent = fmtMb(setup.size); });
    if (typeof setup.digest === 'string' && /^sha256:[0-9a-f]{64}$/.test(setup.digest)) {
      const sha = $('#sha-installer');
      if (sha) sha.textContent = setup.digest.slice(7);
    }
  }
  if (portable) {
    $$('[data-bind="portable-size"]').forEach((el) => { el.textContent = fmtMb(portable.size); });
    if (typeof portable.digest === 'string' && /^sha256:[0-9a-f]{64}$/.test(portable.digest)) {
      const sha = $('#sha-portable');
      if (sha) sha.textContent = portable.digest.slice(7);
    }
  }
}

hydrateGitHub().catch(() => { /* offline or rate-limited: static values stay intact */ });
