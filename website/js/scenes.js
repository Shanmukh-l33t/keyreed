/* Interactive and scroll-scrubbed scenes for Blocks 2–6 and /details code viewer. */

import { gsap, ScrollTrigger, isReducedMotion, sound } from './motion.js';

const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
const sleep = (ms) => new Promise((r) => window.setTimeout(r, ms));

/* =====================================================
   1. Block 2 — Proof band count-up & hairline draw
   ===================================================== */
function initProofBand() {
  const band = $('#proof');
  if (!band) return;
  const nums = $$('[data-count]', band);

  if (isReducedMotion() || !('IntersectionObserver' in window) || !gsap) {
    band.classList.add('in');
    return;
  }

  let ran = false;
  const io = new IntersectionObserver((entries) => {
    if (ran || !entries.some((e) => e.isIntersecting)) return;
    ran = true;
    io.disconnect();
    band.classList.add('in');

    nums.forEach((el) => {
      const target = parseFloat(el.dataset.count || '0');
      const decimals = parseInt(el.dataset.decimals || '0', 10);
      const prefix = el.dataset.prefix || '';
      const start = parseFloat(el.dataset.from || '0');
      const proxy = { v: start };
      gsap.to(proxy, {
        v: target,
        duration: 0.95,
        ease: 'expo.out',
        onUpdate() {
          el.textContent = `${prefix}${proxy.v.toFixed(decimals)}`;
        },
      });
    });
  }, { threshold: 0.25 });

  io.observe(band);
}

/* =====================================================
   2. Block 3 — "Every app. The right voice."
      Real Tier-0 verified examples from crates/vi-core/src/types.rs
   ===================================================== */
const APP_MODES = [
  {
    key: 'professional',
    appKey: 'mail',
    appName: 'Mail',
    modeLabel: 'Professional',
    title: 'New Message — Staging rollout',
    context: 'To: release@acme.test · Subject: Staging rollout',
    raw: 'please revert back once the deployment is completed on staging',
    out: 'Please reply once the deployment is completed on staging.',
  },
  {
    key: 'casual',
    appKey: 'chat',
    appName: 'Chat',
    modeLabel: 'Casual',
    title: '#team-sync — Direct message',
    context: 'Today · 09:42 · Online',
    raw: 'i will be 10 mins late stuck in traffic',
    out: "I'll be 10 mins late stuck in traffic",
  },
  {
    key: 'technical',
    appKey: 'code',
    appName: 'Code editor',
    modeLabel: 'Technical',
    title: 'PR #48 — crates/auth_middleware.rs',
    context: 'Review comment · line 114',
    raw: 'update auth underscore middleware dot rs to return http 401',
    out: 'Update auth_middleware.rs to return HTTP 401.',
  },
  {
    key: 'terminal',
    appKey: 'terminal',
    appName: 'Terminal',
    modeLabel: 'Terminal / Code',
    title: 'pwsh — C:\\dev\\keyreed',
    context: 'PS C:\\dev\\keyreed>',
    raw: 'git checkout dash b fix slash login',
    out: 'git checkout -b fix/login',
  },
  {
    key: 'smart',
    appKey: 'notes',
    appName: 'Notes',
    modeLabel: 'Smart',
    title: 'Sprint notes.md',
    context: 'Documents & notes · Default mode',
    raw: 'um i i think the the build pipeline is stuck on step 3 uh again',
    out: 'I think the build pipeline is stuck on step 3 again.',
  },
  {
    key: 'exact',
    appKey: 'notes',
    appName: 'Protected field',
    modeLabel: 'Exact',
    title: 'Exact dictation',
    context: 'Casing & punctuation only · Never rephrases',
    raw: 'um so the build is stuck on step 3',
    out: 'Um so the build is stuck on step 3.',
  },
];

function initAppsScene() {
  const sec = $('#apps-sec');
  if (!sec) return;

  const pinWrap = $('#apps-pin');
  const win = $('#morph-win');
  const voiceEl = $('#sb-voice');
  const titleEl = $('#mw-title');
  const badgeEl = $('#mw-badge');
  const ctxEl = $('#mw-context');
  const outEl = $('#mw-out');
  const ticks = $$('.rail-tick', sec);
  const chips = $$('.mode-chip', sec);

  let activeIdx = -1;
  let typeRun = 0;

  function applyMode(idx, charRatio = 1, animateSlide = false) {
    const m = APP_MODES[idx] || APP_MODES[0];
    const modeChanged = idx !== activeIdx;
    activeIdx = idx;

    if (modeChanged && win) {
      win.dataset.app = m.appKey;
      if (voiceEl) voiceEl.textContent = `"${m.raw}"`;
      if (titleEl) titleEl.textContent = m.title;
      if (badgeEl) {
        badgeEl.textContent = m.modeLabel;
        if (animateSlide && gsap && !isReducedMotion()) {
          gsap.fromTo(badgeEl, { y: 6, opacity: 0.35 }, { y: 0, opacity: 1, duration: 0.24, ease: 'expo.out' });
        }
      }
      if (ctxEl) ctxEl.textContent = m.context;
      ticks.forEach((t, ti) => t.classList.toggle('active', ti === idx));
      chips.forEach((c) => c.setAttribute('aria-pressed', String(c.dataset.mode === m.key)));
    }

    if (outEl) {
      const chars = Math.max(1, Math.round(m.out.length * Math.min(1, Math.max(0.18, charRatio))));
      outEl.textContent = m.out.slice(0, chars);
    }
  }

  async function selectInteractive(idx) {
    const myType = ++typeRun;
    sound.keyUp();
    if (isReducedMotion()) {
      applyMode(idx, 1, false);
      return;
    }
    applyMode(idx, 0.2, true);
    const full = APP_MODES[idx].out;
    for (let c = 4; c <= full.length; c += 4) {
      if (myType !== typeRun) return;
      if (outEl) outEl.textContent = full.slice(0, c);
      await sleep(16);
    }
    if (outEl) outEl.textContent = full;
  }

  ticks.forEach((t, idx) => {
    t.addEventListener('click', () => selectInteractive(idx));
  });
  chips.forEach((chip) => {
    chip.addEventListener('click', () => {
      const idx = APP_MODES.findIndex((m) => m.key === chip.dataset.mode);
      if (idx >= 0) selectInteractive(idx);
    });
  });

  // Initial state
  applyMode(0, 1, false);

  // Desktop pinned scrubbed scene (>= 768 px, non-reduced-motion)
  if (gsap && ScrollTrigger && pinWrap && !isReducedMotion()) {
    const mm = gsap.matchMedia();
    mm.add('(min-width: 768px)', () => {
      const st = ScrollTrigger.create({
        trigger: pinWrap,
        start: 'top 76px',
        end: '+=240%',
        pin: true,
        scrub: 0.8,
        anticipatePin: 1,
        onUpdate(self) {
          const p = self.progress;
          const stageFloat = Math.min(3.999, p * 4);
          const idx = Math.floor(stageFloat);
          const local = stageFloat - idx;
          const typeProgress = Math.min(1, 0.22 + local * 1.35);
          applyMode(idx, typeProgress, true);
        },
      });
      return () => st.kill();
    });
  }

  // Mobile (< 768 px): horizontal snap carousel gentle auto-advance when visible
  const mobStrip = $('#apps-mobile');
  if (mobStrip && !isReducedMotion() && 'IntersectionObserver' in window) {
    let mobVisible = false;
    let userTouched = false;
    let mobIdx = 0;
    const stopAuto = () => { userTouched = true; };
    mobStrip.addEventListener('pointerdown', stopAuto, { passive: true });
    mobStrip.addEventListener('touchstart', stopAuto, { passive: true });
    mobStrip.addEventListener('focusin', stopAuto);

    const timer = window.setInterval(() => {
      if (!mobVisible || userTouched || window.innerWidth >= 768 || document.hidden) return;
      const cards = $$('.am-card', mobStrip);
      if (!cards.length) return;
      mobIdx = (mobIdx + 1) % cards.length;
      const targetLeft = cards[mobIdx].offsetLeft - mobStrip.offsetLeft;
      mobStrip.scrollTo({ left: targetLeft, behavior: 'smooth' });
    }, 3400);

    const io = new IntersectionObserver((entries) => {
      mobVisible = entries.some((e) => e.isIntersecting);
    }, { threshold: 0.4 });
    io.observe(mobStrip);
    window.addEventListener('pagehide', () => window.clearInterval(timer), { once: true });
  }
}

/* =====================================================
   3. Block 4 — Quick Fix & Composer mini-demos
   ===================================================== */
function initFeaturesScene() {
  // Quick Fix (Ctrl + Win + G)
  const qf = $('#qf-demo');
  if (qf) {
    const words = $$('[data-fix]', qf);
    const keys = $$('.qf-keys kbd', qf);
    const replay = $('#qf-replay');
    let qfToken = 0;

    const resetTypos = () => {
      qf.classList.remove('selecting', 'fixed', 'show-shot');
      words.forEach((w) => { w.textContent = w.dataset.typo || w.textContent; });
      keys.forEach((k) => k.classList.remove('pressed'));
    };

    const showFixedStatic = () => {
      words.forEach((w) => { w.textContent = w.dataset.fix || w.textContent; });
      qf.classList.add('fixed');
    };

    async function playQuickFix() {
      const me = ++qfToken;
      if (isReducedMotion()) {
        showFixedStatic();
        return;
      }
      resetTypos();
      if (replay) replay.textContent = 'Running…';
      await sleep(220);
      if (me !== qfToken) return;

      qf.classList.add('selecting');
      await sleep(480);
      if (me !== qfToken) return;

      for (const k of keys) {
        k.classList.add('pressed');
        await sleep(95);
        if (me !== qfToken) return;
      }
      await sleep(160);
      keys.forEach((k) => k.classList.remove('pressed'));
      if (me !== qfToken) return;

      showFixedStatic();
      await sleep(1350);
      if (me !== qfToken) return;

      qf.classList.add('show-shot');
      if (replay) replay.textContent = 'Replay';
    }

    if (isReducedMotion()) {
      showFixedStatic();
    } else if ('IntersectionObserver' in window) {
      let played = false;
      const io = new IntersectionObserver((entries) => {
        if (played || !entries.some((e) => e.isIntersecting)) return;
        played = true;
        io.disconnect();
        playQuickFix();
      }, { threshold: 0.35 });
      io.observe(qf);
    }

    if (replay) {
      replay.addEventListener('click', () => {
        sound.keyDown();
        playQuickFix();
      });
    }
  }

  // Composer (Ctrl + Win + K) clickable chips + parallax backdrop
  const cmp = $('#cmp-demo');
  if (cmp) {
    const streamEl = $('#cmp-stream');
    const chips = $$('.cmp-chip', cmp);
    const PRESETS = {
      shorter: 'Release moves to Thursday while we finish the design review.',
      warmer: "Quick heads-up — we're moving Thursday's launch so design gets a calm final look.",
      clearer: 'The launch is moving to Thursday because the design review is still in progress.',
    };

    let streamToken = 0;
    async function runComposer(presetKey, animate = true) {
      const me = ++streamToken;
      const text = PRESETS[presetKey] || PRESETS.shorter;
      chips.forEach((c) => c.setAttribute('aria-pressed', String(c.dataset.preset === presetKey)));
      if (!streamEl) return;

      if (!animate || isReducedMotion()) {
        streamEl.textContent = text;
        return;
      }

      streamEl.innerHTML = '';
      const parts = text.split(' ');
      for (const word of parts) {
        if (me !== streamToken) return;
        const s = document.createElement('span');
        s.className = 'sw';
        s.textContent = word;
        streamEl.appendChild(s);
        await sleep(38);
      }
    }

    chips.forEach((chip) => {
      chip.addEventListener('click', () => {
        sound.keyUp();
        runComposer(chip.dataset.preset, true);
      });
    });

    const bgImg = $('#cmp-parallax');
    if (gsap && ScrollTrigger && bgImg && !isReducedMotion()) {
      gsap.fromTo(
        bgImg,
        { y: -10 },
        {
          y: 16,
          ease: 'none',
          scrollTrigger: {
            trigger: cmp,
            start: 'top bottom',
            end: 'bottom top',
            scrub: 0.8,
          },
        }
      );
    }
  }
}

/* =====================================================
   4. Block 5 — "Private by design" slab, particle flow & code receipt
   ===================================================== */
function initPrivacyScene() {
  // Animated emerald particle stream (Mic -> CPU -> Text lines -> App; Cloud stays dark/disconnected)
  const svg = $('#priv-svg');
  if (svg && !isReducedMotion() && 'IntersectionObserver' in window) {
    const dots = $$('.pf-dot', svg);
    const dashes = $$('.pf-dash', svg);
    const cpuPulse = $('#pf-cpu-ring', svg);
    let visible = false;
    let raf = 0;
    const t0 = performance.now();

    const step = (now) => {
      raf = 0;
      if (!visible || document.hidden || isReducedMotion()) return;
      const t = (now - t0) / 1000;

      // Mic (x=68) -> CPU (x=218) audio dots
      dots.forEach((d, idx) => {
        const u = (t * 0.62 + idx / dots.length) % 1;
        const x = 78 + u * 124;
        const y = 66 + Math.sin(u * Math.PI * 3 + idx) * 5;
        const op = Math.sin(u * Math.PI);
        d.setAttribute('transform', `translate(${x.toFixed(1)}, ${y.toFixed(1)})`);
        d.setAttribute('opacity', op.toFixed(2));
      });

      // CPU (x=258) -> App (x=420) text dashes ("voice becomes text")
      dashes.forEach((ln, idx) => {
        const u = (t * 0.58 + idx / dashes.length) % 1;
        const x = 262 + u * 144;
        const y = 62 + (idx % 2) * 8;
        const op = Math.sin(u * Math.PI);
        ln.setAttribute('transform', `translate(${x.toFixed(1)}, ${y.toFixed(1)})`);
        ln.setAttribute('opacity', op.toFixed(2));
      });

      if (cpuPulse) {
        const s = 1 + 0.08 * Math.sin(t * 4.2);
        cpuPulse.setAttribute('transform', `translate(238, 66) scale(${s.toFixed(3)}) translate(-238, -66)`);
      }

      raf = requestAnimationFrame(step);
    };

    const sync = () => {
      if (visible && !document.hidden && !raf) raf = requestAnimationFrame(step);
    };
    const io = new IntersectionObserver((entries) => {
      visible = entries.some((e) => e.isIntersecting);
      sync();
    }, { threshold: 0.1 });
    io.observe(svg);
    document.addEventListener('visibilitychange', sync);
  }

  // Real Rust receipt from window.KR_CODE.privacy (types.rs: MaxPrivacyOffline + allows_network)
  const receiptPre = $('#receipt-pre');
  const src = window.KR_CODE && window.KR_CODE.privacy;
  if (receiptPre && src) {
    // Show the #[default] MaxPrivacyOffline arm + allows_network() method with exact source line numbers
    const sliceIdx = [2, 3, 4, 5, 10, 37, 38, 39, 40];
    receiptPre.innerHTML = sliceIdx.map((idx) => {
      const ln = src.start + idx;
      const isGap = idx === 10;
      if (isGap) return '<span class="l" data-ln="⋮">    <span class="sx-c">// …</span></span>';
      return `<span class="l" data-hl="${src.hl.includes(idx) ? '1' : '0'}" data-ln="${ln}">${src.lines[idx]}</span>`;
    }).join('');

    const hlLines = $$('[data-hl="1"]', receiptPre);
    if (isReducedMotion() || !('IntersectionObserver' in window)) {
      hlLines.forEach((l) => l.classList.add('hl'));
    } else {
      const io = new IntersectionObserver((entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        io.disconnect();
        hlLines.forEach((l, i) => {
          window.setTimeout(() => l.classList.add('hl'), 140 + i * 130);
        });
      }, { threshold: 0.35 });
      io.observe(receiptPre);
    }
  }
}

/* =====================================================
   5. /details full 4-tab KR_CODE source viewer
   ===================================================== */
function initDetailsCodeViewer() {
  const codePre = $('#det-code-pre');
  const codePath = $('#det-code-path');
  const tabs = $$('[data-code-tab]');
  const src = window.KR_CODE;
  if (!codePre || !src || !tabs.length) return;

  function renderFile(key) {
    const f = src[key];
    if (!f) return;
    codePre.innerHTML = f.lines.map((line, i) => {
      const cls = f.hl.includes(i) ? 'l hl' : 'l';
      return `<span class="${cls}" data-ln="${f.start + i}">${line}</span>`;
    }).join('');
    if (codePath) codePath.textContent = f.path;
    tabs.forEach((t) => {
      const on = t.dataset.codeTab === key;
      t.setAttribute('aria-selected', String(on));
      t.tabIndex = on ? 0 : -1;
    });
  }

  renderFile('privacy');
  tabs.forEach((t, idx) => {
    t.addEventListener('click', () => renderFile(t.dataset.codeTab));
    t.addEventListener('keydown', (e) => {
      let next = -1;
      if (e.key === 'ArrowRight') next = (idx + 1) % tabs.length;
      else if (e.key === 'ArrowLeft') next = (idx - 1 + tabs.length) % tabs.length;
      if (next < 0) return;
      e.preventDefault();
      tabs[next].focus();
      renderFile(tabs[next].dataset.codeTab);
    });
  });
}

initProofBand();
initAppsScene();
initFeaturesScene();
initPrivacyScene();
initDetailsCodeViewer();
