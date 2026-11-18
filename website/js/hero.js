/* Hero interactive 3D Right Ctrl toy, intro choreography, SplitText filler cleanup,
   curved Bezier text flight, real ControlRight detection, and Finale reeds mount. */

import { gsap, SplitText, isReducedMotion, sound } from './motion.js';
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

function initHero() {
  const heroCanvas = document.getElementById('hero-reeds');
  const finaleCanvas = document.getElementById('finale-reeds');
  const heroReeds = heroCanvas ? mountReeds(heroCanvas, { calm: false }) : null;
  if (finaleCanvas) mountReeds(finaleCanvas, { calm: true });

  // 404 playful keycap handler if on 404 page
  const nfKey = document.getElementById('nf-keycap');
  const nfStatus = document.getElementById('nf-status');
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

  const toy = document.getElementById('hero-toy');
  const keycap = document.getElementById('hero-keycap');
  if (!toy || !keycap) return;

  const barsWrap = document.getElementById('lp-bars');
  const statusEl = document.getElementById('lp-status');
  const rawEl = document.getElementById('raw-stream');
  const mailTyped = document.getElementById('hm-typed');
  const orb = document.getElementById('flight-orb');
  const liveEl = document.getElementById('toy-live');
  const replayBtn = document.getElementById('toy-replay');

  // Build 13 live waveform bars
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

  // Intro choreography (<= 1.6 s total; H1 stays visible from CSS first paint)
  if (gsap && !isReducedMotion()) {
    gsap.fromTo(
      keycap,
      { opacity: 0, y: -18, rotateX: 24, scale: 0.92 },
      { opacity: 1, y: 0, rotateX: 0, scale: 1, duration: 0.72, delay: 0.32, ease: 'back.out(1.6)', clearProps: 'transform,opacity' }
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
    sound.whooshStart();
    const tick = (now) => {
      if (!isHolding) return;
      const t = (now - tStart) / 1000;
      // Speech-like syllable cadence
      const syllable = 0.45 + 0.35 * Math.sin(t * 9.2) + 0.2 * Math.sin(t * 21.5);
      const env = Math.max(0.15, Math.min(1, syllable));
      if (heroReeds) heroReeds.setVoice(env);
      sound.whooshLevel(env);
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

    // Ensure full phrase is visible even if user tapped briefly
    const heldMs = performance.now() - holdStartedAt;
    while (wordIndex < RAW_WORDS.length) {
      appendNextWord();
      if (heldMs < 500) await sleep(10);
      if (myRun !== runToken) return;
    }

    // Phase 2: Filler strike-through & collapse ("um", "uh") + bars -> 3 text lines
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

    // Phase 3: Curved motion path from raw stream into floating mail window
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

    // Phase 4: Type clean sentence at caret + draw check mark
    for (let i = 7; i <= CLEAN_TEXT.length; i += 7) {
      mailTyped.textContent = CLEAN_TEXT.slice(0, i);
      await sleep(14);
      if (myRun !== runToken) return;
    }
    mailTyped.textContent = CLEAN_TEXT;
    setPhase('landed', 'Inserted');
    sound.chime();
    if (liveEl) liveEl.textContent = `Clean text inserted: ${CLEAN_TEXT}`;
  }

  // Scripted single auto-play if nobody interacts within 1.4 s of load (never loops!)
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

  // Pointer interaction on 3D keycap
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

  // Keyboard Space / Enter on focused keycap button
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

  // Real physical Right Ctrl held solo > 250 ms anywhere on the page;
  // immediately cancelled if any other key joins so Ctrl+C / shortcuts work untouched!
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

    // Another key pressed! Abort pending or active ControlRight capture so Ctrl+C etc. work cleanly.
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

  // Replay button
  if (replayBtn) {
    replayBtn.addEventListener('click', async () => {
      userInteracted = true;
      if (isHolding) await finishHold(true);
      if (isReducedMotion()) {
        renderCompletedStatic();
        return;
      }
      userInteracted = false; // allow scripted sequence once
      runAutoPlayOnce();
      userInteracted = true;
    });
  }
}

initHero();
