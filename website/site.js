/* KeyReed website: navigation, reveal-on-scroll, FAQ, copy buttons, release info, source viewer.
   No dependencies. Everything degrades to a fully readable static page without JavaScript. */
(() => {
  'use strict';
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* ---------- Theme toggle (light is the default) ---------- */
  const root = document.documentElement;
  const themeBtn = $('#theme-btn');
  const metaTheme = document.querySelector('meta[name="theme-color"]');
  const paintTheme = (t) => {
    root.setAttribute('data-theme', t);
    const next = t === 'dark' ? 'light' : 'dark';
    themeBtn.setAttribute('aria-label', `Switch to ${next} theme`);
    themeBtn.title = `Switch to ${next} theme`;
    if (metaTheme) metaTheme.setAttribute('content', t === 'dark' ? '#060908' : '#f6f9f6');
  };
  paintTheme(root.getAttribute('data-theme') === 'dark' ? 'dark' : 'light');
  themeBtn.addEventListener('click', () => {
    const t = root.getAttribute('data-theme') === 'dark' ? 'light' : 'dark';
    if (!reduce) { root.classList.add('theming'); setTimeout(() => root.classList.remove('theming'), 450); }
    paintTheme(t);
    try { localStorage.setItem('keyreed-theme', t); } catch { /* storage blocked */ }
  });

  /* ---------- Header ---------- */
  const nav = $('#nav');
  const menuBtn = $('#menu-btn');
  const mobileNav = $('#mobile-nav');
  const syncNav = () => nav.classList.toggle('scrolled', scrollY > 8 || !mobileNav.hidden);
  addEventListener('scroll', syncNav, { passive: true });
  syncNav();

  const setMenu = (open) => {
    menuBtn.setAttribute('aria-expanded', String(open));
    menuBtn.setAttribute('aria-label', open ? 'Close menu' : 'Open menu');
    menuBtn.querySelector('use').setAttribute('href', open ? '#i-plus' : '#i-menu');
    mobileNav.hidden = !open;
    syncNav();
  };
  menuBtn.addEventListener('click', () => setMenu(menuBtn.getAttribute('aria-expanded') !== 'true'));
  mobileNav.addEventListener('click', (e) => { if (e.target.closest('a')) setMenu(false); });
  addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !mobileNav.hidden) { setMenu(false); menuBtn.focus(); }
  });
  matchMedia('(min-width: 1021px)').addEventListener('change', (e) => { if (e.matches) setMenu(false); });

  // Highlight the nav link of the section in view.
  const spyLinks = new Map($$('[data-spy]').map((a) => [a.dataset.spy, a]));
  if ('IntersectionObserver' in window) {
    const spy = new IntersectionObserver((entries) => {
      for (const e of entries) {
        const link = spyLinks.get(e.target.id);
        if (link) link.classList.toggle('active', e.isIntersecting);
      }
    }, { rootMargin: '-40% 0px -55% 0px' });
    spyLinks.forEach((_, id) => { const s = document.getElementById(id); if (s) spy.observe(s); });
  }

  /* ---------- Reveal on scroll ---------- */
  const targets = $$('[data-reveal], [data-stagger]');
  $$('[data-stagger]').forEach((g) => Array.from(g.children).forEach((c, i) => c.style.setProperty('--i', i)));
  $$('.cmp tbody tr').forEach((r, i) => r.style.setProperty('--i', i));
  if ('IntersectionObserver' in window && !reduce) {
    const io = new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); }
      }
    }, { threshold: 0.14, rootMargin: '0px 0px -6% 0px' });
    targets.forEach((t) => io.observe(t));
  } else {
    targets.forEach((t) => t.classList.add('in'));
  }

  /* ---------- Cursor-responsive border lighting ---------- */
  if (matchMedia('(hover: hover)').matches && !reduce) {
    let raf = 0, el = null, px = 0, py = 0;
    document.addEventListener('pointermove', (e) => {
      const t = e.target.closest && e.target.closest('.lit');
      if (!t) return;
      el = t; px = e.clientX; py = e.clientY;
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        const r = el.getBoundingClientRect();
        el.style.setProperty('--mx', (px - r.left) + 'px');
        el.style.setProperty('--my', (py - r.top) + 'px');
      });
    }, { passive: true });
  }

  /* ---------- FAQ accordion ---------- */
  $$('.qa button').forEach((b) => {
    b.addEventListener('click', () => {
      const open = b.getAttribute('aria-expanded') === 'true';
      b.setAttribute('aria-expanded', String(!open));
      b.closest('.qa').classList.toggle('open', !open);
    });
  });

  /* ---------- Copy buttons ---------- */
  $$('[data-copy-from]').forEach((b) => {
    const original = b.innerHTML;
    b.addEventListener('click', async () => {
      const text = document.getElementById(b.dataset.copyFrom).textContent.trim();
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        const ta = document.createElement('textarea');
        ta.value = text; ta.setAttribute('readonly', ''); ta.style.position = 'fixed'; ta.style.opacity = '0';
        document.body.appendChild(ta); ta.select();
        try { document.execCommand('copy'); } catch { /* nothing else to try */ }
        ta.remove();
      }
      b.classList.add('done');
      b.innerHTML = '<svg class="ic"><use href="#i-check"/></svg>';
      setTimeout(() => { b.classList.remove('done'); b.innerHTML = original; }, 1600);
    });
  });

  /* ---------- Live release information ----------
     Download links use GitHub's stable /releases/latest/download/<asset> URLs, so they never go stale.
     This only refreshes the version, sizes and checksums shown next to them. On any failure the
     values written in the HTML stay in place. */
  const REPO = 'Shanmukh-l33t/keyreed';
  const fmtSize = (n) => (n >= 1e9 ? (n / 1e9).toFixed(2) + ' GB' : (n / 1e6).toFixed(1) + ' MB');
  const setText = (sel, text) => $$(sel).forEach((n) => { n.textContent = text; });

  async function hydrateRelease() {
    const key = 'keyreed-release-v1';
    let rel = null;
    try {
      const c = JSON.parse(sessionStorage.getItem(key) || 'null');
      if (c && Date.now() - c.t < 3600e3) rel = c.d;
    } catch { /* ignore */ }
    if (!rel) {
      const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: 'application/vnd.github+json' } });
      if (!r.ok) return;
      const j = await r.json();
      rel = { tag: j.tag_name, assets: (j.assets || []).map((a) => ({ name: a.name, size: a.size, digest: a.digest })) };
      try { sessionStorage.setItem(key, JSON.stringify({ t: Date.now(), d: rel })); } catch { /* ignore */ }
    }
    const ver = String(rel.tag || '').replace(/^v/, '');
    if (!/^\d+\.\d+\.\d+/.test(ver)) return;
    setText('#pill-ver', `KeyReed ${ver}`);
    setText('#dl-ver', `Version ${ver}`);
    const find = (name) => rel.assets.find((a) => a.name === name);
    const setup = find('KeyReed-Setup-x64.exe');
    const full = rel.assets.find((a) => a.name.includes('full-offline') || a.name === 'KeyReed-Full-x64.zip');
    if (setup) {
      setText('[data-bind="installer-size"]', fmtSize(setup.size));
      if (setup.digest && /^sha256:[0-9a-f]{64}$/.test(setup.digest)) setText('#sha-installer', setup.digest.slice(7));
    }
    if (full) {
      setText('[data-bind="full-size"]', fmtSize(full.size));
      const dlFull = $('#dl-full');
      if (dlFull && full.name) dlFull.href = `https://github.com/${REPO}/releases/latest/download/${full.name}`;
      setText('#full-name', full.name);
      if (full.digest && /^sha256:[0-9a-f]{64}$/.test(full.digest)) setText('#sha-full', full.digest.slice(7));
    }
  }
  hydrateRelease().catch(() => { /* offline or rate limited: keep the static values */ });

  /* ---------- Source viewer (excerpts generated from the real repository) ---------- */
  const codePre = $('#code-pre');
  const codeBody = $('#code-body');
  const codePath = $('#code-path');
  const tabs = $$('.code-tabs [role="tab"]');
  const SRC = window.KR_CODE;

  function showFile(key, animate = true) {
    const f = SRC && SRC[key];
    if (!f) return;
    codePre.innerHTML = f.lines.map((l, i) => {
      const cls = f.hl.includes(i) ? 'l hl' : 'l';
      return `<span class="${cls}" style="--i:${animate && !reduce ? i : 0}">${l}</span>`;
    }).join('');
    codePre.style.counterReset = `ln ${f.start - 1}`;
    codePath.textContent = f.path;
    tabs.forEach((t) => {
      const on = t.dataset.file === key;
      t.setAttribute('aria-selected', String(on));
      t.tabIndex = on ? 0 : -1;
      if (on) codeBody.setAttribute('aria-labelledby', t.id);
    });
  }
  if (SRC && codePre) {
    // Counters on the pseudo-elements read from the code element, so set the start there.
    let started = false;
    const start = () => { if (!started) { started = true; showFile('privacy'); } };
    if ('IntersectionObserver' in window) {
      const cio = new IntersectionObserver((es) => { if (es.some((e) => e.isIntersecting)) { start(); cio.disconnect(); } }, { threshold: 0.25 });
      cio.observe($('#code'));
    } else { start(); }
    tabs.forEach((t, i) => {
      t.addEventListener('click', () => { started = true; showFile(t.dataset.file); });
      t.addEventListener('keydown', (e) => {
        let n = -1;
        if (e.key === 'ArrowRight') n = (i + 1) % tabs.length;
        else if (e.key === 'ArrowLeft') n = (i - 1 + tabs.length) % tabs.length;
        else if (e.key === 'Home') n = 0;
        else if (e.key === 'End') n = tabs.length - 1;
        if (n < 0) return;
        e.preventDefault(); tabs[n].focus(); started = true; showFile(tabs[n].dataset.file);
      });
    });
  }
})();
