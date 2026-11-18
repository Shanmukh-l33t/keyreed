/* Hero interactive 3D Right Ctrl toy, playful cursor acoustic reed-field, 3D specular keycap tilt,
   SplitText filler cleanup, curved Bezier text flight, real ControlRight detection, and Finale reeds. */

import { gsap, SplitText, isReducedMotion, hasFinePointer, sound } from './motion.js';
import { mountReeds } from './reeds.js';

const RAW_WORDS = [
  { w: 'um', filler: true },
  { w: 'so' },
  { w: 'the' },
  { w: 'launch' },
  { w: 'is' },
  { w: 'uh', filler: true },
  { w: 'moving' },
  { w: 'to' },
  { w: 'thursday' },
  { w: 'because' },
  { w: 'the' },
  { w: 'design' },
  { w: 'review' },
  { w: "isn't" },
  { w: 'done' },
  { w: 'yet' },
];

const CLEAN_TEXT = "So the launch is moving to Thursday because the design review isn't done yet.";
const sleep = (ms) => new Promise((r) => window.setTimeout(r, ms));

/* =====================================================
   Interactive Cursor Acoustic Reed-Field (#hero-field)
   Micro-ticks wake into emerald waveform bars along your cursor's path
   with liquid velocity wakes and click/keypress shockwave rings.
   Sleeps at 0% CPU when settled or offscreen.
   ===================================================== */
function mountHeroField(canvas) {
  const api = { emitFromEl: () => {}, emitAt: () => {} };
  if (!canvas || isReducedMotion() || !hasFinePointer()) return api;

  const host = canvas.closest('.hero-open') || canvas.parentElement;
  if (!host) return api;
  const ctx = canvas.getContext('2d');
  if (!ctx) return api;

  const STEP = 40;
  let width = 0, height = 0, dpr = 1;
  let cols = 0, rows = 0;
  let nodes = [];
  const ripples = [];

  let ptrX = -9999, ptrY = -9999;
  let smoothX = -9999, smoothY = -9999;
  let velX = 0, velY = 0;
  let ptrInside = false;
  let visible = true;
  let raf = 0;

  function buildGrid() {
    dpr = Math.min(window.devicePixelRatio || 1, 1.5);
    width = Math.max(1, host.clientWidth);
    height = Math.max(1, host.clientHeight);
    canvas.width = Math.floor(width * dpr);
    canvas.height = Math.floor(height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    cols = Math.ceil(width / STEP) + 1;
    rows = Math.ceil(height / STEP) + 1;
    const ox = (width - (cols - 1) * STEP) * 0.5;
    const oy = (height - (rows - 1) * STEP) * 0.5;
    nodes = [];
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        nodes.push({
          x: ox + c * STEP,
          y: oy + r * STEP,
          ang: -Math.PI * 0.5,
          energy: 0,
        });
      }
    }
    drawStaticIdle();
  }

  function isDarkTheme() {
    return document.documentElement.getAttribute('data-theme') === 'dark';
  }

  function drawStaticIdle() {
    ctx.clearRect(0, 0, width, height);
    const dark = isDarkTheme();
    ctx.fillStyle = dark ? 'rgba(167, 243, 208, 0.12)' : 'rgba(11, 59, 44, 0.11)';
    for (let i = 0; i < nodes.length; i++) {
      const n = nodes[i];
      ctx.beginPath();
      ctx.arc(n.x, n.y, 1.25, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  function wake() {
    if (!raf && visible && !document.hidden && !isReducedMotion()) {
      raf = requestAnimationFrame(render);
    }
  }

  function render() {
    raf = 0;
    if (!visible || document.hidden || isReducedMotion()) return;

    const dark = isDarkTheme();
    ctx.clearRect(0, 0, width, height);

    if (ptrInside) {
      if (smoothX < -900) { smoothX = ptrX; smoothY = ptrY; }
      const dx = ptrX - smoothX;
      const dy = ptrY - smoothY;
      velX = velX * 0.78 + dx * 0.22;
      velY = velY * 0.78 + dy * 0.22;
      smoothX += dx * 0.28;
      smoothY += dy * 0.28;

      // Soft ambient emerald cursor spotlight
      const grad = ctx.createRadialGradient(smoothX, smoothY, 0, smoothX, smoothY, 230);
      grad.addColorStop(0, dark ? 'rgba(52, 211, 153, 0.11)' : 'rgba(16, 185, 129, 0.085)');
      grad.addColorStop(1, 'rgba(16, 185, 129, 0)');
      ctx.fillStyle = grad;
      ctx.fillRect(smoothX - 240, smoothY - 240, 480, 480);
    } else {
      velX *= 0.85;
      velY *= 0.85;
    }

    // Advance acoustic ripples
    for (let i = ripples.length - 1; i >= 0; i--) {
      const rp = ripples[i];
      rp.r += 9.2;
      rp.life *= 0.962;
      if (rp.life < 0.02 || rp.r > Math.max(width, height) * 1.2) {
        ripples.splice(i, 1);
      } else {
        ctx.strokeStyle = dark
          ? `rgba(110, 231, 183, ${(rp.life * 0.22).toFixed(3)})`
          : `rgba(16, 185, 129, ${(rp.life * 0.18).toFixed(3)})`;
        ctx.lineWidth = 1.2;
        ctx.beginPath();
        ctx.arc(rp.x, rp.y, rp.r, 0, Math.PI * 2);
        ctx.stroke();
      }
    }

    const speed = Math.hypot(velX, velY);
    const velAng = speed > 0.4 ? Math.atan2(velY, velX) : null;
    const RADIUS = 205;
    const R2 = RADIUS * RADIUS;
    let maxEnergy = ripples.length > 0 ? 1 : 0;

    ctx.lineCap = 'round';
    const baseDotFill = dark ? 'rgba(167, 243, 208, 0.12)' : 'rgba(11, 59, 44, 0.11)';

    for (let i = 0; i < nodes.length; i++) {
      const n = nodes[i];
      let targetE = 0;
      let targetAng = n.ang;

      if (ptrInside) {
        const dx = n.x - smoothX;
        const dy = n.y - smoothY;
        const d2 = dx * dx + dy * dy;
        if (d2 < R2) {
          const dist = Math.sqrt(d2);
          const prox = Math.pow(1 - dist / RADIUS, 1.55);
          const boost = Math.min(1.25, 0.55 + speed * 0.045);
          targetE = Math.min(1, prox * boost);
          const radialAng = Math.atan2(dy, dx) + Math.PI * 0.5;
          targetAng = velAng !== null ? radialAng * 0.45 + velAng * 0.55 : radialAng;
        }
      }

      for (let r = 0; r < ripples.length; r++) {
        const rp = ripples[r];
        const dx = n.x - rp.x;
        const dy = n.y - rp.y;
        const dist = Math.hypot(dx, dy);
        const band = Math.abs(dist - rp.r);
        if (band < 58) {
          const crest = (1 - band / 58) * rp.life * rp.strength;
          if (crest > targetE) {
            targetE = Math.min(1, crest);
            targetAng = Math.atan2(dy, dx);
          }
        }
      }

      if (targetE > n.energy) {
        n.energy += (targetE - n.energy) * 0.34;
      } else {
        n.energy *= 0.925;
      }

      if (targetE > 0.02) {
        // Shortest angular interpolation
        let diff = ((targetAng - n.ang + Math.PI * 3) % (Math.PI * 2)) - Math.PI;
        n.ang += diff * 0.22;
      }

      if (n.energy > maxEnergy) maxEnergy = n.energy;

      if (n.energy < 0.025) {
        ctx.fillStyle = baseDotFill;
        ctx.beginPath();
        ctx.arc(n.x, n.y, 1.25, 0, Math.PI * 2);
        ctx.fill();
      } else {
        const len = 2.5 + n.energy * 15.5;
        const hx = Math.cos(n.ang) * len * 0.5;
        const hy = Math.sin(n.ang) * len * 0.5;
        const alpha = (dark ? 0.22 : 0.18) + n.energy * (dark ? 0.72 : 0.68);
        ctx.strokeStyle = dark
          ? `rgba(52, 211, 153, ${alpha.toFixed(3)})`
          : `rgba(5, 150, 105, ${alpha.toFixed(3)})`;
        ctx.lineWidth = 1.6 + n.energy * 1.15;
        ctx.beginPath();
        ctx.moveTo(n.x - hx, n.y - hy);
        ctx.lineTo(n.x + hx, n.y + hy);
        ctx.stroke();
      }
    }

    if (ptrInside || maxEnergy > 0.008) {
      raf = requestAnimationFrame(render);
    }
  }

  api.emitAt = (clientX, clientY, strength = 0.95) => {
    const r = host.getBoundingClientRect();
    ripples.push({
      x: clientX - r.left,
      y: clientY - r.top,
      r: 14,
      life: 1,
      strength,
    });
    if (ripples.length > 5) ripples.shift();
    wake();
  };

  api.emitFromEl = (el, strength = 0.95) => {
    if (!el) return;
    const r = el.getBoundingClientRect();
    api.emitAt(r.left + r.width * 0.5, r.top + r.height * 0.5, strength);
  };

  host.addEventListener('pointermove', (e) => {
    const r = host.getBoundingClientRect();
    ptrX = e.clientX - r.left;
    ptrY = e.clientY - r.top;
    ptrInside = true;
    wake();
  }, { passive: true });

  host.addEventListener('pointerleave', () => {
    ptrInside = false;
    wake();
  }, { passive: true });

  host.addEventListener('pointerdown', (e) => {
    if (e.target.closest('a, button')) return;
    api.emitAt(e.clientX, e.clientY, 0.85);
  }, { passive: true });

  const observer = new MutationObserver(() => drawStaticIdle());
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });

  if ('IntersectionObserver' in window) {
    const io = new IntersectionObserver((entries) => {
      visible = entries.some((en) => en.isIntersecting);
      if (visible && (ptrInside || ripples.length)) wake();
    }, { threshold: 0.02 });
    io.observe(host);
  }

  window.addEventListener('resize', buildGrid, { passive: true });
  if ('requestIdleCallback' in window) {
    window.requestIdleCallback(buildGrid, { timeout: 250 });
  } else {
    window.setTimeout(buildGrid, 40);
  }

  return api;
}

/* =====================================================
   3D Specular Cursor-Tracking on Keycap + H1 Word Float
   ===================================================== */
function attachPlayfulCursorCraft(keycapEl) {
  if (isReducedMotion() || !hasFinePointer()) return;

  // 1. 3D keycap cursor tilt & specular reflection
  if (keycapEl) {
    const host = keycapEl.closest('.hero-open') || document.body;
    let tx = 0, ty = 0, cx = 0, cy = 0, raf = 0;

    const tick = () => {
      raf = 0;
      cx += (tx - cx) * 0.16;
      cy += (ty - cy) * 0.16;
      keycapEl.style.setProperty('--rx', `${cx.toFixed(2)}deg`);
      keycapEl.style.setProperty('--ry', `${cy.toFixed(2)}deg`);
      if (Math.abs(tx - cx) > 0.05 || Math.abs(ty - cy) > 0.05) {
        raf = requestAnimationFrame(tick);
      }
    };

    host.addEventListener('pointermove', (e) => {
      const r = keycapEl.getBoundingClientRect();
      if (!r.width) return;
      const kx = Math.max(0, Math.min(100, ((e.clientX - r.left) / r.width) * 100));
      const ky = Math.max(0, Math.min(100, ((e.clientY - r.top) / r.height) * 100));
      keycapEl.style.setProperty('--kx', `${kx.toFixed(1)}%`);
      keycapEl.style.setProperty('--ky', `${ky.toFixed(1)}%`);

      if (keycapEl.classList.contains('is-held')) {
        tx = 0; ty = 0;
      } else {
        const dx = Math.max(-1, Math.min(1, (e.clientX - (r.left + r.width / 2)) / 280));
        const dy = Math.max(-1, Math.min(1, (e.clientY - (r.top + r.height / 2)) / 240));
        tx = -dy * 8.5;
        ty = dx * 10.5;
      }
      if (!raf) raf = requestAnimationFrame(tick);
    }, { passive: true });

    host.addEventListener('pointerleave', () => {
      tx = 0; ty = 0;
      if (!raf) raf = requestAnimationFrame(tick);
    }, { passive: true });
  }

  // 2. Tactile spring micro-float on individual Hero H1 words
  const heroWords = document.querySelectorAll('.h-hero .hw');
  heroWords.forEach((word) => {
    word.addEventListener('pointermove', (e) => {
      const r = word.getBoundingClientRect();
      const dx = ((e.clientX - (r.left + r.width / 2)) / Math.max(1, r.width)) * 6;
      const dy = ((e.clientY - (r.top + r.height / 2)) / Math.max(1, r.height)) * 4 - 3.5;
      word.style.transition = 'transform 160ms cubic-bezier(.2,.8,.2,1)';
      word.style.transform = `translate3d(${dx.toFixed(1)}px, ${dy.toFixed(1)}px, 0)`;
    });
    word.addEventListener('pointerleave', () => {
      word.style.transition = 'transform 420ms cubic-bezier(.16,1,.3,1)';
      word.style.transform = '';
    });
  });
}

function initHero() {
  const heroCanvas = document.getElementById('hero-reeds');
  const finaleCanvas = document.getElementById('finale-reeds');
  const fieldCanvas = document.getElementById('hero-field');
  const heroReeds = heroCanvas ? mountReeds(heroCanvas, { calm: false }) : null;
  if (finaleCanvas) mountReeds(finaleCanvas, { calm: true });
  const heroField = mountHeroField(fieldCanvas);

  // 404 playful keycap handler if on 404 page
  const nfKey = document.getElementById('nf-keycap');
  const nfStatus = document.getElementById('nf-status');
  const toy = document.getElementById('hero-toy');
  const keycap = document.getElementById('hero-keycap');

  attachPlayfulCursorCraft(keycap || nfKey);

  if (nfKey) {
    const msgs = [
      'Key not bound — nothing at this URL.',
      'Checked every shortcut. Still 404.',
      'Try heading back to KeyReed home.',
    ];
    let idx = 0;
    const pressNf = () => {
      nfKey.setAttribute('aria-pressed', 'true');
      nfKey.classList.add('is-held');
      sound.keyDown();
      heroField.emitFromEl(nfKey, 1.0);
      if (heroReeds) heroReeds.setVoice(0.75);
      if (nfStatus) nfStatus.textContent = msgs[idx++ % msgs.length];
    };
    const releaseNf = () => {
      if (nfKey.getAttribute('aria-pressed') !== 'true') return;
      nfKey.setAttribute('aria-pressed', 'false');
      nfKey.classList.remove('is-held');
      sound.keyUp();
      if (heroReeds) heroReeds.setVoice(0);
    };
    nfKey.addEventListener('pointerdown', pressNf);
    window.addEventListener('pointerup', releaseNf);
    nfKey.addEventListener('keydown', (e) => {
      if ((e.key === ' ' || e.key === 'Enter') && !e.repeat) { e.preventDefault(); pressNf(); }
    });
    nfKey.addEventListener('keyup', (e) => {
      if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); releaseNf(); }
    });
  }

  if (!toy || !keycap) return;

  const barsWrap = document.getElementById('lp-bars');
  const statusEl = document.getElementById('lp-status');
  const rawEl = document.getElementById('raw-stream');
  const mailTyped = document.getElementById('hm-typed');
  const orb = document.getElementById('flight-orb');
  const liveEl = document.getElementById('toy-live');
  const replayBtn = document.getElementById('toy-replay');

  const BAR_COUNT = 13;
  const bars = [];
  if (barsWrap) {
    barsWrap.innerHTML = '';
    for (let i = 0; i < BAR_COUNT; i++) {
      const b = document.createElement('i');
      barsWrap.appendChild(b);
      bars.push(b);
    }
  }

  if (gsap && !isReducedMotion()) {
    gsap.fromTo(
      keycap,
      { opacity: 0, y: -18, scale: 0.92 },
      { opacity: 1, y: 0, scale: 1, duration: 0.72, delay: 0.32, ease: 'back.out(1.6)', clearProps: 'transform,opacity' }
    );
    gsap.fromTo(
      '.hero-ctas',
      { opacity: 0, y: 14 },
      { opacity: 1, y: 0, duration: 0.68, delay: 0.46, ease: 'expo.out', clearProps: 'transform,opacity' }
    );
  }

  let userInteracted = false;
  let runToken = 0;
  let isHolding = false;
  let holdSource = null;
  let holdStartedAt = 0;
  let waveRaf = 0;
  let wordIndex = 0;
  let wordTimer = 0;

  function setPhase(phase, label) {
    toy.dataset.phase = phase;
    if (statusEl) statusEl.textContent = label;
  }

  function renderCompletedStatic() {
    rawEl.innerHTML = RAW_WORDS.map(({ w, filler }) => (
      `<span class="rw${filler ? ' filler strike' : ''}">${w}</span>`
    )).join(' ');
    mailTyped.textContent = CLEAN_TEXT;
    setPhase('landed', 'Inserted');
  }

  if (isReducedMotion()) {
    renderCompletedStatic();
  }

  function stopWave() {
    if (waveRaf) {
      cancelAnimationFrame(waveRaf);
      waveRaf = 0;
    }
    bars.forEach((b) => { b.style.transform = 'scaleY(0.18)'; });
    if (heroReeds) heroReeds.setVoice(0);
    sound.whooshStop();
  }

  function startWave() {
    if (isReducedMotion()) return;
    const tStart = performance.now();
    let lastPulse = 0;
    sound.whooshStart();
    const tick = (now) => {
      if (!isHolding) return;
      const t = (now - tStart) / 1000;
      const syllable = 0.45 + 0.35 * Math.sin(t * 9.2) + 0.2 * Math.sin(t * 21.5);
      const env = Math.max(0.15, Math.min(1, syllable));
      if (heroReeds) heroReeds.setVoice(env);
      sound.whooshLevel(env);
      if (now - lastPulse > 340) {
        lastPulse = now;
        heroField.emitFromEl(keycap, 0.62);
      }
      bars.forEach((b, idx) => {
        const centerDist = Math.abs(idx - (BAR_COUNT - 1) / 2) / ((BAR_COUNT - 1) / 2);
        const bell = 1 - centerDist * 0.62;
        const local = 0.2 + bell * env * (0.65 + 0.35 * Math.sin(t * 15 + idx * 1.1));
        b.style.transform = `scaleY(${Math.min(1, local).toFixed(2)})`;
      });
      waveRaf = requestAnimationFrame(tick);
    };
    waveRaf = requestAnimationFrame(tick);
  }

  function appendNextWord() {
    if (wordIndex >= RAW_WORDS.length) return;
    const item = RAW_WORDS[wordIndex++];
    const span = document.createElement('span');
    span.className = `rw${item.filler ? ' filler' : ''}`;
    span.textContent = item.w;
    rawEl.appendChild(span);
    if (gsap && !isReducedMotion()) {
      gsap.fromTo(span, { opacity: 0, y: 5 }, { opacity: 1, y: 0, duration: 0.2, ease: 'expo.out' });
    }
  }

  function beginHold(source) {
    userInteracted = true;
    if (isHolding) return;
    const myRun = ++runToken;
    isHolding = true;
    holdSource = source;
    holdStartedAt = performance.now();

    keycap.setAttribute('aria-pressed', 'true');
    keycap.classList.add('is-held');
    sound.keyDown();
    heroField.emitFromEl(keycap, 1.0);

    if (isReducedMotion()) {
      rawEl.innerHTML = '';
      RAW_WORDS.forEach(() => appendNextWord());
      mailTyped.textContent = '';
      setPhase('listening', 'Listening…');
      return;
    }

    rawEl.innerHTML = '';
    mailTyped.textContent = '';
    wordIndex = 0;
    setPhase('listening', 'Listening…');
    startWave();

    const stepWord = () => {
      if (myRun !== runToken || !isHolding) return;
      appendNextWord();
      if (wordIndex < RAW_WORDS.length) {
        wordTimer = window.setTimeout(stepWord, 125);
      }
    };
    stepWord();
  }

  async function finishHold(aborted = false) {
    if (!isHolding) return;
    const myRun = ++runToken;
    isHolding = false;
    holdSource = null;
    window.clearTimeout(wordTimer);

    keycap.setAttribute('aria-pressed', 'false');
    keycap.classList.remove('is-held');
    sound.keyUp();
    stopWave();

    if (aborted) {
      renderCompletedStatic();
      return;
    }

    if (isReducedMotion()) {
      renderCompletedStatic();
      if (liveEl) liveEl.textContent = `Clean text inserted: ${CLEAN_TEXT}`;
      return;
    }

    const heldMs = performance.now() - holdStartedAt;
    while (wordIndex < RAW_WORDS.length) {
      appendNextWord();
      if (heldMs < 500) await sleep(10);
      if (myRun !== runToken) return;
    }

    setPhase('cleaning', 'Voice becomes text…');
    const fillers = Array.from(rawEl.querySelectorAll('.rw.filler'));
    fillers.forEach((f) => f.classList.add('strike'));

    if (SplitText && gsap && fillers.length) {
      const splits = fillers.map((el) => new SplitText(el, { type: 'chars' }));
      const chars = splits.flatMap((s) => s.chars);
      gsap.to(chars, {
        opacity: 0,
        scale: 0.4,
        y: -4,
        stagger: 0.018,
        duration: 0.18,
        ease: 'power2.in',
      });
    }

    await sleep(200);
    if (myRun !== runToken) return;
    fillers.forEach((f) => f.classList.add('collapsed'));

    if (gsap && orb) {
      const stageRect = toy.querySelector('.toy-stage').getBoundingClientRect();
      const fromRect = rawEl.getBoundingClientRect();
      const toRect = mailTyped.parentElement.getBoundingClientRect();

      const x0 = fromRect.left - stageRect.left + fromRect.width * 0.35;
      const y0 = fromRect.top - stageRect.top + fromRect.height * 0.5;
      const x2 = toRect.left - stageRect.left + 28;
      const y2 = toRect.top - stageRect.top + 22;
      const cx = (x0 + x2) * 0.5 + 48;
      const cy = Math.min(y0, y2) - 44;

      const proxy = { t: 0 };
      orb.style.opacity = '1';
      await new Promise((resolve) => {
        gsap.to(proxy, {
          t: 1,
          duration: 0.34,
          ease: 'power2.inOut',
          onUpdate() {
            const u = 1 - proxy.t;
            const x = u * u * x0 + 2 * u * proxy.t * cx + proxy.t * proxy.t * x2;
            const y = u * u * y0 + 2 * u * proxy.t * cy + proxy.t * proxy.t * y2;
            const sc = 0.85 + 0.25 * Math.sin(proxy.t * Math.PI);
            orb.style.transform = `translate3d(${x.toFixed(1)}px, ${y.toFixed(1)}px, 0) scale(${sc.toFixed(2)})`;
          },
          onComplete: resolve,
        });
      });
      orb.style.opacity = '0';
      if (myRun !== runToken) return;
    }

    for (let i = 7; i <= CLEAN_TEXT.length; i += 7) {
      mailTyped.textContent = CLEAN_TEXT.slice(0, i);
      await sleep(14);
      if (myRun !== runToken) return;
    }
    mailTyped.textContent = CLEAN_TEXT;
    setPhase('landed', 'Inserted');
    heroField.emitFromEl(mailTyped, 0.75);
    sound.chime();
    if (liveEl) liveEl.textContent = `Clean text inserted: ${CLEAN_TEXT}`;
  }

  async function runAutoPlayOnce() {
    if (userInteracted || isReducedMotion() || document.hidden) return;
    const myRun = ++runToken;
    isHolding = true;
    holdSource = 'auto';
    holdStartedAt = performance.now();
    keycap.classList.add('is-held');
    rawEl.innerHTML = '';
    mailTyped.textContent = '';
    wordIndex = 0;
    setPhase('listening', 'Listening…');
    startWave();

    while (wordIndex < RAW_WORDS.length) {
      if (userInteracted || myRun !== runToken) return;
      appendNextWord();
      await sleep(115);
    }
    await sleep(220);
    if (userInteracted || myRun !== runToken) return;
    await finishHold(false);
  }

  if (!isReducedMotion()) {
    window.setTimeout(() => {
      if (!userInteracted) runAutoPlayOnce();
    }, 1400);
  }

  keycap.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    beginHold('pointer');
  });
  window.addEventListener('pointerup', () => {
    if (holdSource === 'pointer') finishHold(false);
  });
  window.addEventListener('pointercancel', () => {
    if (holdSource === 'pointer') finishHold(false);
  });

  keycap.addEventListener('keydown', (e) => {
    if ((e.key === ' ' || e.key === 'Enter') && !e.repeat) {
      e.preventDefault();
      beginHold('button-key');
    }
  });
  keycap.addEventListener('keyup', (e) => {
    if ((e.key === ' ' || e.key === 'Enter') && holdSource === 'button-key') {
      e.preventDefault();
      finishHold(false);
    }
  });

  let rightCtrlTimer = 0;
  let rightCtrlPending = false;

  window.addEventListener('keydown', (e) => {
    if (e.code === 'ControlRight') {
      if (e.repeat || rightCtrlPending || isHolding) return;
      if (e.altKey || e.shiftKey || e.metaKey) return;
      rightCtrlPending = true;
      rightCtrlTimer = window.setTimeout(() => {
        if (!rightCtrlPending) return;
        rightCtrlPending = false;
        beginHold('ControlRight');
      }, 255);
      return;
    }

    if (rightCtrlPending) {
      rightCtrlPending = false;
      window.clearTimeout(rightCtrlTimer);
    }
    if (holdSource === 'ControlRight') {
      finishHold(true);
    }
  });

  window.addEventListener('keyup', (e) => {
    if (e.code === 'ControlRight') {
      if (rightCtrlPending) {
        rightCtrlPending = false;
        window.clearTimeout(rightCtrlTimer);
      }
      if (holdSource === 'ControlRight') {
        finishHold(false);
      }
    }
  });

  window.addEventListener('blur', () => {
    if (rightCtrlPending) {
      rightCtrlPending = false;
      window.clearTimeout(rightCtrlTimer);
    }
    if (isHolding && holdSource !== 'auto') finishHold(false);
  });

  if (replayBtn) {
    replayBtn.addEventListener('click', async () => {
      userInteracted = true;
      if (isHolding) await finishHold(true);
      if (isReducedMotion()) {
        renderCompletedStatic();
        return;
      }
      userInteracted = false;
      runAutoPlayOnce();
      userInteracted = true;
    });
  }
}

initHero();
