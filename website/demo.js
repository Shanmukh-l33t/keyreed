/* KeyReed website: the animated dictation illustration and the writing-mode explorer.
   Everything here is a scripted illustration. It never touches the microphone.
   The scene only runs while it is on screen and the tab is visible. */
(() => {
  'use strict';
  const $ = (id) => document.getElementById(id);
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  /* =====================================================
     Hero scene: hold, speak, release, typed into the focused field (Auto Insert, the default)
     ===================================================== */
  const scene = $('scene');
  if (scene) {
    const el = {
      state: $('kr-state'), wave: $('kr-wave'), bubble: $('bubble'), btext: $('bubble-text'), btag: $('bubble-tag'),
      mailBody: $('mail-body'), mailText: $('mail-text'), keyCtrl: $('key-ctrl'), keyPaste: $('key-paste'),
      steps: Array.from(document.querySelectorAll('.rail-step')),
    };

    // Waveform bars. Heights animate with transform only; the bell-shaped envelope looks like speech.
    const BARS = 26;
    for (let i = 0; i < BARS; i++) {
      const b = document.createElement('i');
      const bell = Math.pow(Math.sin(Math.PI * (i + 0.5) / BARS), 0.8);
      b.style.setProperty('--amp', (0.3 + 0.7 * bell * (0.6 + 0.4 * Math.random())).toFixed(2));
      b.style.setProperty('--dur', (0.45 + Math.random() * 0.5).toFixed(2) + 's');
      b.style.setProperty('--del', (-Math.random()).toFixed(2) + 's');
      el.wave.appendChild(b);
    }

    // What "was said" and what KeyReed "wrote". Words flagged junk are removed by the cleanup step.
    const SAID = [
      ['hey'], ['maya'], ['so', 1], ['the'], ['launch'], ['is'], ['moving'], ['to'], ['thursday'], ['um', 1],
      ['because'], ['the'], ['design'], ['review'], ['is'], ['not'], ['finished'], ['yet'],
      ['can'], ['you'], ['update'], ['the', 1], ['the'], ['calendar'], ['invite'],
    ];
    const WROTE = 'Hey Maya, the launch is moving to Thursday because the design review is not finished yet. Can you update the calendar invite?';

    const setState = (s, label) => { scene.dataset.state = s; el.state.textContent = label; };
    const setStep = (n) => el.steps.forEach((s, i) => {
      const k = i + 1;
      s.classList.toggle('on', k === n);
      s.classList.toggle('done', k < n);
    });
    const setBubble = (show, tag, out) => {
      el.bubble.classList.toggle('show', show);
      el.bubble.classList.toggle('out', !!out);
      if (tag) el.btag.textContent = tag;
    };

    const reset = () => {
      setState('idle', 'Hold Right Ctrl');
      setStep(0);
      setBubble(false, 'Heard');
      el.btext.textContent = '';
      el.keyCtrl.classList.remove('down');
      el.keyPaste.classList.remove('down');
      el.mailText.textContent = '';
      el.mailText.classList.remove('flash');
      el.mailBody.classList.remove('filled');
    };

    const showFinal = () => {
      setState('ready', 'Inserted');
      setStep(4);
      setBubble(true, 'KeyReed wrote', true);
      el.btext.textContent = WROTE;
      el.mailText.textContent = WROTE;
      el.mailBody.classList.add('filled');
    };

    let run = 0;
    async function play() {
      const me = ++run;
      const ok = () => me === run;
      reset();
      await sleep(1000); if (!ok()) return;

      // 1. Hold
      el.keyCtrl.classList.add('down'); setStep(1);
      await sleep(550); if (!ok()) return;

      // 2. Speak: the microphone is open, bars move, words arrive
      setState('listening', 'Listening'); setStep(2);
      setBubble(true, 'Heard');
      el.btext.textContent = '';
      const spans = SAID.map(([w, junk]) => {
        const s = document.createElement('span');
        s.className = 'wd' + (junk ? ' junk' : '');
        s.textContent = w;
        return s;
      });
      for (const s of spans) {
        el.btext.appendChild(s);
        await sleep(165); if (!ok()) return;
      }
      await sleep(500); if (!ok()) return;

      // 3. Release: transcribe and clean up
      el.keyCtrl.classList.remove('down');
      setState('processing', 'Transcribing and correcting'); setStep(3);
      await sleep(650); if (!ok()) return;
      el.btext.querySelectorAll('.junk').forEach((s) => s.classList.add('cut'));
      await sleep(1000); if (!ok()) return;
      el.btext.textContent = WROTE;
      el.btext.classList.remove('fadein'); void el.btext.offsetWidth; el.btext.classList.add('fadein');
      setBubble(true, 'KeyReed wrote', true);

      // 4. Typed into the field the cursor was in. The real app types at about 600 characters a second.
      setStep(4);
      el.mailBody.classList.add('filled');
      for (let i = 0; i <= WROTE.length; i += 6) {
        el.mailText.textContent = WROTE.slice(0, i);
        await sleep(16); if (!ok()) return;
      }
      el.mailText.textContent = WROTE;
      setState('ready', 'Inserted');
      await sleep(4200); if (!ok()) return;
      setBubble(false);
      await sleep(800); if (!ok()) return;
      play(); // loop
    }

    const stop = () => { run++; };

    if (reduce) {
      showFinal();
      const rp = $('replay'); if (rp) rp.hidden = true;
    } else if ('IntersectionObserver' in window) {
      let visible = false;
      const sync = () => { if (visible && !document.hidden) { if (!scene.dataset.running) { scene.dataset.running = '1'; play(); } } else { stop(); delete scene.dataset.running; } };
      new IntersectionObserver((es) => { visible = es[0].isIntersecting; sync(); }, { threshold: 0.25 }).observe(scene);
      document.addEventListener('visibilitychange', sync);
      const rp = $('replay');
      if (rp) rp.addEventListener('click', () => { scene.dataset.running = '1'; play(); });
    } else {
      play();
    }
  }

  /* =====================================================
     Writing-mode explorer
     ===================================================== */
  const MODES = {
    smart: {
      title: 'Smart',
      desc: 'Natural, clear sentences without stiff formality. Removes fillers and stutters and resolves spoken self-corrections.',
      raw: 'um can we push the review to tuesday sorry no wait wednesday the the design is almost done',
      out: 'Can we push the review to Wednesday? The design is almost done.',
      auto: 'Used automatically in <b>documents and notes</b>. It is also the default mode.',
    },
    exact: {
      title: 'Exact',
      desc: 'Casing and punctuation only. No filler removal and no rephrasing, so what you said is what you get.',
      raw: 'um so the the draft is ready for review',
      out: 'Um so the the draft is ready for review.',
      auto: 'Used automatically in <b>password-protected windows</b>.',
    },
    professional: {
      title: 'Professional',
      desc: 'Concise, natural tone for workplace email and documents.',
      raw: 'hey can you send me the report by friday i need it for the client call',
      out: 'Could you send me the report by Friday? I need it for the client call.',
      auto: 'Used automatically in <b>mail</b>.',
    },
    casual: {
      title: 'Casual',
      desc: 'Relaxed chat tone. Keeps contractions and brevity and never over-formalizes.',
      raw: 'yeah im running like ten minutes late grab me a seat',
      out: "Yeah, I'm running like ten minutes late, grab me a seat.",
      auto: 'Used automatically in <b>chat</b>.',
    },
    technical: {
      title: 'Technical',
      desc: 'Preserves every identifier, acronym, version, path and number, and applies canonical casing such as API, OAuth and PostgreSQL.',
      raw: 'the api returns a 404 when the v2 endpoint is missing from the oauth config',
      out: 'The API returns a 404 when the v2 endpoint is missing from the OAuth config.',
      auto: 'Used automatically in <b>code editors</b>.',
    },
    terminal: {
      title: 'Terminal / Code',
      desc: 'Compiles spoken flags, pipes and paths into command syntax. Never improves a command, and warns before destructive ones.',
      raw: 'docker logs dash f web pipe grep error',
      out: 'docker logs -f web | grep error',
      auto: 'Used automatically in <b>terminal windows</b>. Spoken symbols such as "forward slash" apply only here.',
    },
  };

  const panel = $('mode-panel');
  if (panel) {
    const tabs = Array.from(document.querySelectorAll('#mode-tabs [role="tab"]'));
    const rawEl = $('mp-raw'), outEl = $('mp-out');
    const norm = (w) => w.toLowerCase().replace(/[^a-z0-9']/g, '');

    // Word-level LCS so we can show what changed: struck words on the left, highlighted words on the right.
    function diff(rawWords, outWords) {
      const a = rawWords.map(norm), b = outWords.map(norm);
      const dp = Array.from({ length: a.length + 1 }, () => new Array(b.length + 1).fill(0));
      for (let i = a.length - 1; i >= 0; i--) for (let j = b.length - 1; j >= 0; j--) {
        dp[i][j] = a[i] && a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
      }
      const keptRaw = new Set(), keptOut = new Set();
      let i = 0, j = 0;
      while (i < a.length && j < b.length) {
        if (a[i] && a[i] === b[j]) { keptRaw.add(i); keptOut.add(j); i++; j++; }
        else if (dp[i + 1][j] >= dp[i][j + 1]) i++;
        else j++;
      }
      return { keptRaw, keptOut };
    }

    let token = 0;
    async function show(key, animate) {
      const m = MODES[key];
      const me = ++token;
      const ok = () => me === token;
      $('mp-title').textContent = m.title;
      $('mp-desc').textContent = m.desc;
      $('mp-auto').innerHTML = m.auto;
      const rawWords = m.raw.split(' ');
      const outWords = m.out.split(' ');
      const { keptRaw, keptOut } = diff(rawWords, outWords);
      rawEl.textContent = ''; outEl.textContent = '';

      const rawSpans = rawWords.map((w) => { const s = document.createElement('span'); s.textContent = w + ' '; return s; });
      const renderOut = (delay) => {
        outEl.textContent = '';
        outWords.forEach((w, i) => {
          const s = document.createElement('span');
          s.className = 'wd' + (!keptOut.has(i) ? ' chg' : '');
          s.textContent = w;
          s.style.animationDelay = (i * delay) + 'ms';
          outEl.appendChild(s);
        });
        // A kept word whose exact text changed (casing, punctuation, symbols) is also highlighted.
        let r = 0;
        outWords.forEach((w, i) => {
          if (keptOut.has(i)) {
            while (!keptRaw.has(r)) r++;
            if (rawWords[r] !== w) outEl.children[i].classList.add('chg');
            r++;
          }
        });
      };

      if (!animate || reduce) {
        rawSpans.forEach((s, i) => { if (!keptRaw.has(i)) s.className = 'del'; rawEl.appendChild(s); });
        renderOut(0);
        return;
      }
      for (const s of rawSpans) { rawEl.appendChild(s); await sleep(34); if (!ok()) return; }
      await sleep(420); if (!ok()) return;
      rawSpans.forEach((s, i) => { if (!keptRaw.has(i)) s.className = 'del'; });
      renderOut(48);
    }

    function select(tab, animate = true, focus = false) {
      tabs.forEach((t) => {
        const on = t === tab;
        t.setAttribute('aria-selected', String(on));
        t.tabIndex = on ? 0 : -1;
      });
      panel.setAttribute('aria-labelledby', tab.id);
      if (focus) tab.focus();
      show(tab.dataset.mode, animate);
    }

    tabs.forEach((t, i) => {
      t.addEventListener('click', () => select(t));
      t.addEventListener('keydown', (e) => {
        const vertical = matchMedia('(min-width: 861px)').matches;
        const next = vertical ? 'ArrowDown' : 'ArrowRight', prev = vertical ? 'ArrowUp' : 'ArrowLeft';
        let n = -1;
        if (e.key === next || e.key === 'ArrowRight' || e.key === 'ArrowDown') n = (i + 1) % tabs.length;
        else if (e.key === prev || e.key === 'ArrowLeft' || e.key === 'ArrowUp') n = (i - 1 + tabs.length) % tabs.length;
        else if (e.key === 'Home') n = 0;
        else if (e.key === 'End') n = tabs.length - 1;
        if (n < 0) return;
        e.preventDefault();
        select(tabs[n], true, true);
      });
    });

    // Fill the panel immediately so it is never empty, then play the first animation when it scrolls into view.
    show('smart', false);
    if ('IntersectionObserver' in window && !reduce) {
      const io = new IntersectionObserver((es) => {
        if (es.some((e) => e.isIntersecting)) {
          io.disconnect();
          const active = tabs.find((t) => t.getAttribute('aria-selected') === 'true');
          show(active.dataset.mode, true);
        }
      }, { threshold: 0.4 });
      io.observe(panel);
    }
  }
})();
