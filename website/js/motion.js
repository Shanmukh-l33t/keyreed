/* Motion tokens, Lenis smooth scroll, GSAP + ScrollTrigger + SplitText setup,
   magnetic buttons, cursor-lit surfaces, and opt-in WebAudio synthesis. */

export const MOTION = Object.freeze({
  easeOutCss: 'cubic-bezier(.16,1,.3,1)',
  easeIoCss: 'cubic-bezier(.65,0,.35,1)',
  easeBrandCss: 'cubic-bezier(.2,.8,.2,1)',
  easeOut: 'expo.out',
  easeIo: 'power3.inOut',
  micro: 0.2,
  reveal: 1.0,
  stagger: 0.07,
  scrub: 0.8,
});

const rmQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
const fineQuery = window.matchMedia('(hover: hover) and (pointer: fine)');

export const isReducedMotion = () => rmQuery.matches;
export const hasFinePointer = () => fineQuery.matches;

export const gsap = window.gsap || null;
export const ScrollTrigger = window.ScrollTrigger || null;
export const SplitText = window.SplitText || null;

if (gsap && ScrollTrigger) gsap.registerPlugin(ScrollTrigger);
if (gsap && SplitText) gsap.registerPlugin(SplitText);

let lenis = null;
let lastInputWasKeyboard = false;

window.addEventListener('keydown', (e) => {
  if (e.key === 'Tab' || e.key.startsWith('Arrow') || e.key === 'PageDown' || e.key === 'PageUp' || e.key === 'Home' || e.key === 'End') {
    lastInputWasKeyboard = true;
  }
}, { passive: true });
window.addEventListener('pointerdown', () => { lastInputWasKeyboard = false; }, { passive: true });
window.addEventListener('wheel', () => { lastInputWasKeyboard = false; }, { passive: true });

export function initMotion() {
  const root = document.documentElement;
  rmQuery.addEventListener('change', (e) => {
    root.classList.toggle('rm', e.matches);
    if (e.matches && lenis) {
      lenis.destroy();
      lenis = null;
    }
  });

  // Smooth scroll: desktop with fine pointer only, off for reduced motion and touch.
  if (window.Lenis && gsap && ScrollTrigger && hasFinePointer() && !isReducedMotion() && window.innerWidth >= 768) {
    lenis = new window.Lenis({
      duration: 1.05,
      easing: (t) => Math.min(1, 1.001 - Math.pow(2, -10 * t)),
      smoothWheel: true,
      syncTouch: false,
      autoRaf: false,
    });
    lenis.on('scroll', ScrollTrigger.update);
    gsap.ticker.add((time) => {
      if (lenis) lenis.raf(time * 1000);
    });
    gsap.ticker.lagSmoothing(0);
  }

  // Smooth in-page anchor navigation that respects keyboard-only jumps and reduced motion.
  document.addEventListener('click', (e) => {
    const link = e.target.closest && e.target.closest('a[href^="#"]');
    if (!link) return;
    const hash = link.getAttribute('href');
    if (!hash || hash === '#') return;
    const target = document.getElementById(hash.slice(1));
    if (!target) return;
    if (lenis && !isReducedMotion() && !lastInputWasKeyboard) {
      e.preventDefault();
      history.pushState(null, '', hash);
      lenis.scrollTo(target, { offset: -68, duration: 0.9 });
    }
  });

  initCursorLight();
  initMagneticButtons();
  initReveals();
}

export function setScrollLocked(locked) {
  document.documentElement.classList.toggle('nav-locked', locked);
  if (lenis) {
    if (locked) lenis.stop();
    else lenis.start();
  }
}

export function withWillChange(el, prop = 'transform, opacity') {
  if (!el) return () => {};
  el.style.willChange = prop;
  return () => { el.style.willChange = ''; };
}

function initCursorLight() {
  if (!hasFinePointer() || isReducedMotion()) return;
  let raf = 0, active = null, px = 0, py = 0;
  document.addEventListener('pointermove', (e) => {
    const card = e.target.closest && e.target.closest('.lit');
    if (!card) return;
    active = card;
    px = e.clientX;
    py = e.clientY;
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      if (!active) return;
      const r = active.getBoundingClientRect();
      active.style.setProperty('--mx', `${(px - r.left).toFixed(1)}px`);
      active.style.setProperty('--my', `${(py - r.top).toFixed(1)}px`);
    });
  }, { passive: true });
}

function initMagneticButtons() {
  if (!hasFinePointer() || isReducedMotion()) return;
  const buttons = document.querySelectorAll('.btn-primary, .mag');
  buttons.forEach((btn) => {
    let rect = null;
    btn.addEventListener('pointerenter', () => {
      rect = btn.getBoundingClientRect();
      btn.style.willChange = 'transform';
    });
    btn.addEventListener('pointermove', (e) => {
      if (!rect || isReducedMotion()) return;
      const dx = (e.clientX - (rect.left + rect.width / 2)) / (rect.width / 2);
      const dy = (e.clientY - (rect.top + rect.height / 2)) / (rect.height / 2);
      const tx = Math.max(-6, Math.min(6, dx * 5.5));
      const ty = Math.max(-6, Math.min(6, dy * 4.5));
      btn.style.transform = `translate3d(${tx.toFixed(2)}px, ${ty.toFixed(2)}px, 0)`;
    });
    const reset = () => {
      rect = null;
      btn.style.transform = '';
      btn.style.willChange = '';
    };
    btn.addEventListener('pointerleave', reset);
    btn.addEventListener('blur', reset);
  });
}

function initReveals() {
  const groups = document.querySelectorAll('[data-stagger]');
  groups.forEach((g) => {
    Array.from(g.children).forEach((child, idx) => {
      child.style.setProperty('--i', String(idx));
    });
  });

  const targets = document.querySelectorAll('[data-reveal], [data-stagger]');
  if (!targets.length) return;

  if (isReducedMotion() || !('IntersectionObserver' in window)) {
    targets.forEach((t) => t.classList.add('in'));
    return;
  }

  const io = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const el = entry.target;
      el.style.willChange = 'transform, opacity';
      el.classList.add('in');
      setTimeout(() => { el.style.willChange = ''; }, 1250);
      io.unobserve(el);
    }
  }, { threshold: 0.12, rootMargin: '0px 0px -6% 0px' });

  targets.forEach((t) => io.observe(t));
}

/* ---------- Runtime Synthesised WebAudio (zero audio files, off by default) ---------- */
class SoundEngine {
  constructor() {
    this.ctx = null;
    this.whooshGain = null;
    this.whooshFilter = null;
    this.whooshSrc = null;
  }

  isEnabled() {
    return document.documentElement.getAttribute('data-sound') === 'on';
  }

  setEnabled(on) {
    const next = on ? 'on' : 'off';
    document.documentElement.setAttribute('data-sound', next);
    try { localStorage.setItem('keyreed-sound', next); } catch { /* storage blocked */ }
    if (!on) this.whooshStop();
    else this.keyUp();
  }

  ensure() {
    if (!this.isEnabled()) return null;
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return null;
    if (!this.ctx) this.ctx = new AC();
    if (this.ctx.state === 'suspended') this.ctx.resume().catch(() => {});
    return this.ctx;
  }

  noiseBuffer(ctx, seconds = 0.5) {
    const len = Math.floor(ctx.sampleRate * seconds);
    const buf = ctx.createBuffer(1, len, ctx.sampleRate);
    const data = buf.getChannelData(0);
    let last = 0;
    for (let i = 0; i < len; i++) {
      const white = Math.random() * 2 - 1;
      last = (last + 0.04 * white) / 1.04;
      data[i] = last * 2.4;
    }
    return buf;
  }

  thock(down = true) {
    const ctx = this.ensure();
    if (!ctx) return;
    const now = ctx.currentTime;

    // Tactile housing thock
    const osc = ctx.createOscillator();
    const g = ctx.createGain();
    osc.type = 'triangle';
    osc.frequency.setValueAtTime(down ? 162 : 205, now);
    osc.frequency.exponentialRampToValueAtTime(down ? 58 : 92, now + 0.055);
    g.gain.setValueAtTime(0.0001, now);
    g.gain.linearRampToValueAtTime(down ? 0.085 : 0.055, now + 0.004);
    g.gain.exponentialRampToValueAtTime(0.0001, now + 0.068);
    osc.connect(g).connect(ctx.destination);
    osc.start(now);
    osc.stop(now + 0.075);

    // Subtle keycap click transient
    const src = ctx.createBufferSource();
    src.buffer = this.noiseBuffer(ctx, 0.04);
    const bp = ctx.createBiquadFilter();
    bp.type = 'bandpass';
    bp.frequency.value = down ? 1350 : 1850;
    bp.Q.value = 1.6;
    const ng = ctx.createGain();
    ng.gain.setValueAtTime(down ? 0.045 : 0.032, now);
    ng.gain.exponentialRampToValueAtTime(0.0001, now + 0.028);
    src.connect(bp).connect(ng).connect(ctx.destination);
    src.start(now);
    src.stop(now + 0.035);
  }

  keyDown() { this.thock(true); }
  keyUp() { this.thock(false); }

  whooshStart() {
    const ctx = this.ensure();
    if (!ctx) return;
    this.whooshStop();
    const now = ctx.currentTime;
    const src = ctx.createBufferSource();
    src.buffer = this.noiseBuffer(ctx, 1.5);
    src.loop = true;
    const lp = ctx.createBiquadFilter();
    lp.type = 'lowpass';
    lp.frequency.setValueAtTime(320, now);
    lp.Q.value = 0.9;
    const g = ctx.createGain();
    g.gain.setValueAtTime(0.0001, now);
    g.gain.linearRampToValueAtTime(0.016, now + 0.18);
    src.connect(lp).connect(g).connect(ctx.destination);
    src.start(now);
    this.whooshSrc = src;
    this.whooshFilter = lp;
    this.whooshGain = g;
  }

  whooshLevel(amp = 0.3) {
    if (!this.ctx || !this.whooshGain || !this.whooshFilter) return;
    const now = this.ctx.currentTime;
    const clamped = Math.max(0, Math.min(1, amp));
    this.whooshFilter.frequency.setTargetAtTime(280 + clamped * 520, now, 0.06);
    this.whooshGain.gain.setTargetAtTime(0.008 + clamped * 0.028, now, 0.06);
  }

  whooshStop() {
    if (!this.ctx || !this.whooshSrc) return;
    try {
      const now = this.ctx.currentTime;
      this.whooshGain.gain.setTargetAtTime(0.0001, now, 0.08);
      const s = this.whooshSrc;
      setTimeout(() => { try { s.stop(); s.disconnect(); } catch {} }, 220);
    } catch {}
    this.whooshSrc = null;
    this.whooshGain = null;
    this.whooshFilter = null;
  }

  chime() {
    const ctx = this.ensure();
    if (!ctx) return;
    const now = ctx.currentTime;
    [523.25, 783.99].forEach((freq, idx) => {
      const osc = ctx.createOscillator();
      const g = ctx.createGain();
      osc.type = 'sine';
      const t0 = now + idx * 0.065;
      osc.frequency.setValueAtTime(freq, t0);
      g.gain.setValueAtTime(0.0001, t0);
      g.gain.linearRampToValueAtTime(idx === 0 ? 0.042 : 0.034, t0 + 0.018);
      g.gain.exponentialRampToValueAtTime(0.0001, t0 + 0.48);
      osc.connect(g).connect(ctx.destination);
      osc.start(t0);
      osc.stop(t0 + 0.52);
    });
  }
}

export const sound = new SoundEngine();
