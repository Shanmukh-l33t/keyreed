/* Hand-written WebGL2 instanced emerald reed field ("Reeds that hear you").
   Zero dependencies. Starts after first paint, pauses off-screen, falls back to reeds-poster.webp. */

import { isReducedMotion } from './motion.js';

const VERT = `#version 300 es
precision highp float;

layout(location = 0) in vec2 aSeg;      // x: -1..1 across blade width, y: 0..1 base->tip
layout(location = 1) in vec4 aInst;     // x: posX (-1.08..1.08), y: depth (0..1), z: height, w: phase

uniform float uTime;
uniform float uAspect;
uniform float uGrow;
uniform vec3 uPointer;   // x: ndcX, y: ndcY, z: strength
uniform vec2 uVoice;     // x: amplitude (0..1), y: ripple clock

out float vT;
out float vDepth;
out float vGlow;
out float vEdge;

float hash1(float n) {
  return fract(sin(n * 127.1) * 43758.5453123);
}

float vnoise(float x) {
  float i = floor(x);
  float f = fract(x);
  float u = f * f * (3.0 - 2.0 * f);
  return mix(hash1(i), hash1(i + 1.0), u);
}

void main() {
  float t = aSeg.y;
  float bx = aInst.x;
  float depth = aInst.y;
  float h = aInst.z;
  float ph = aInst.w;

  // Left-to-right growth stagger during intro choreography
  float x01 = clamp((bx + 1.08) / 2.16, 0.0, 1.0);
  float grow = clamp(uGrow * 1.42 - x01 * 0.42, 0.0, 1.0);
  grow = 1.0 - pow(1.0 - grow, 3.0);

  // Layered sine breeze + value noise gust
  float gust = vnoise(bx * 2.4 - uTime * 0.38 + ph * 0.2) - 0.5;
  float sway = sin(uTime * 1.15 + bx * 2.6 + ph) * 0.036
             + sin(uTime * 2.05 - bx * 1.4 + ph * 1.7) * 0.015
             + gust * 0.042;

  // Pointer wind repulsion with smooth Gaussian falloff
  float dx = bx - uPointer.x;
  float pDist = abs(dx);
  float repel = exp(-pDist * pDist * 11.0) * uPointer.z;
  float dir = dx >= 0.0 ? 1.0 : -1.0;
  sway += dir * repel * 0.13;

  // Outward sound-wave rings when voice plays
  float distCenter = abs(bx);
  float waveFront = sin(distCenter * 13.0 - uVoice.y * 7.2);
  float ringEnv = exp(-distCenter * 1.25) * uVoice.x;
  float ripplePush = (bx >= 0.0 ? 1.0 : -1.0) * (0.5 + 0.5 * waveFront) * ringEnv * 0.085;
  sway += ripplePush;

  // Natural arch + quadratic bend along blade height
  float naturalLean = (hash1(ph * 13.7) - 0.5) * 0.075;
  float bend = (naturalLean + sway) * (t * t);

  // Tapered blade silhouette with soft feathered edge (zero sub-pixel aliasing)
  float baseHalfW = mix(0.0055, 0.0105, depth) / max(uAspect * 0.58, 0.65);
  float taper = (1.0 - pow(t, 1.45)) * sin(clamp(t * 3.05 + 0.22, 0.0, 3.14159));
  float halfW = baseHalfW * max(taper, 0.18);

  float depthYOffset = mix(-0.98, -1.04, depth);
  float bladeH = h * mix(0.68, 1.28, depth) * grow;

  float px = bx + bend + aSeg.x * halfW;
  float py = depthYOffset + t * bladeH - abs(bend) * 0.16 * t;

  gl_Position = vec4(px, py, 0.0, 1.0);
  vT = t;
  vDepth = depth;
  vGlow = clamp(ringEnv * (0.55 + 0.45 * waveFront) + repel * 0.35, 0.0, 1.2);
  vEdge = aSeg.x;
}
`;

const FRAG = `#version 300 es
precision mediump float;

in float vT;
in float vDepth;
in float vGlow;
in float vEdge;

uniform float uVoiceAmp;
out vec4 outColor;

void main() {
  // Brand palette: evergreen #0B3B2C -> emerald #10B981 / #34D399 -> mint #A7F3D0
  vec3 cBase = vec3(0.035, 0.165, 0.120);
  vec3 cMid  = vec3(0.063, 0.725, 0.506);
  vec3 cTip  = vec3(0.655, 0.953, 0.816);

  vec3 col = mix(cBase, cMid, smoothstep(0.06, 0.68, vT));
  col = mix(col, cTip, smoothstep(0.62, 1.0, vT));

  // Tip luminescence boosted by voice envelope & ripple ring
  float tipBoost = pow(vT, 2.4) * (0.30 + vGlow * 0.85 + uVoiceAmp * 0.45);
  col += vec3(0.20, 0.82, 0.60) * tipBoost;

  // Depth fog into dark stage floor (#060B09)
  vec3 fog = vec3(0.024, 0.043, 0.035);
  float fogAmt = mix(0.62, 0.04, vDepth);
  col = mix(col, fog, fogAmt * (1.0 - vT * 0.35));

  float edgeSoft = smoothstep(1.0, 0.28, abs(vEdge));
  float alpha = mix(0.45, 0.96, vDepth) * smoothstep(0.0, 0.08, vT) * edgeSoft;
  outColor = vec4(col, alpha);
}
`;

function compileShader(gl, type, src) {
  const sh = gl.createShader(type);
  gl.shaderSource(sh, src);
  gl.compileShader(sh);
  if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
    gl.deleteShader(sh);
    return null;
  }
  return sh;
}

export function mountReeds(canvas, options = {}) {
  const api = {
    setVoice: () => {},
    triggerGrow: () => {},
    destroy: () => {},
  };
  if (!canvas || isReducedMotion()) return api;

  const bed = canvas.closest('.reeds-bed');
  const calm = Boolean(options.calm);

  const boot = () => {
    if (isReducedMotion()) return;
    const gl = canvas.getContext('webgl2', {
      alpha: true,
      antialias: true,
      depth: false,
      stencil: false,
      powerPreference: 'low-power',
      premultipliedAlpha: false,
    });
    if (!gl) return;

    const vs = compileShader(gl, gl.VERTEX_SHADER, VERT);
    const fs = compileShader(gl, gl.FRAGMENT_SHADER, FRAG);
    if (!vs || !fs) return;

    const prog = gl.createProgram();
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.linkProgram(prog);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return;

    // Blade ribbon geometry (6 vertical segments -> 14 vertices in TRIANGLE_STRIP)
    const SEGS = 6;
    const segVerts = new Float32Array((SEGS + 1) * 2 * 2);
    let ptr = 0;
    for (let i = 0; i <= SEGS; i++) {
      const y = i / SEGS;
      segVerts[ptr++] = -1; segVerts[ptr++] = y;
      segVerts[ptr++] =  1; segVerts[ptr++] = y;
    }

    const mobile = window.innerWidth < 768;
    const count = calm ? (mobile ? 320 : 720) : (mobile ? 600 : 1600);
    const inst = new Float32Array(count * 4);

    // Deterministic PRNG so layout is lush & balanced every visit
    let seed = calm ? 9137 : 4219;
    const rand = () => {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      return seed / 4294967296;
    };

    const rows = [];
    for (let i = 0; i < count; i++) {
      const bx = (rand() * 2 - 1) * 1.08;
      const depth = Math.pow(rand(), 0.92);
      // Slight valley right under center text, taller frame toward flanks
      const edgeLift = 0.82 + 0.32 * Math.min(1, Math.abs(bx) * 1.15);
      const height = (0.42 + rand() * 0.62) * edgeLift * (calm ? 0.82 : 1.0);
      const phase = rand() * Math.PI * 2;
      rows.push({ bx, depth, height, phase });
    }
    rows.sort((a, b) => a.depth - b.depth);
    rows.forEach((r, i) => {
      const o = i * 4;
      inst[o] = r.bx;
      inst[o + 1] = r.depth;
      inst[o + 2] = r.height;
      inst[o + 3] = r.phase;
    });

    const vao = gl.createVertexArray();
    gl.bindVertexArray(vao);

    const segBuf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, segBuf);
    gl.bufferData(gl.ARRAY_BUFFER, segVerts, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

    const instBuf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, instBuf);
    gl.bufferData(gl.ARRAY_BUFFER, inst, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 4, gl.FLOAT, false, 0, 0);
    gl.vertexAttribDivisor(1, 1);

    gl.bindVertexArray(null);

    const loc = {
      uTime: gl.getUniformLocation(prog, 'uTime'),
      uAspect: gl.getUniformLocation(prog, 'uAspect'),
      uGrow: gl.getUniformLocation(prog, 'uGrow'),
      uPointer: gl.getUniformLocation(prog, 'uPointer'),
      uVoice: gl.getUniformLocation(prog, 'uVoice'),
      uVoiceAmp: gl.getUniformLocation(prog, 'uVoiceAmp'),
    };

    let aspect = 1;
    const resize = () => {
      const dpr = Math.min(window.devicePixelRatio || 1, 1.75);
      const w = Math.max(1, Math.floor(canvas.clientWidth * dpr));
      const h = Math.max(1, Math.floor(canvas.clientHeight * dpr));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
      aspect = w / Math.max(1, h);
    };
    resize();
    window.addEventListener('resize', resize, { passive: true });

    // Pointer wind tracking on parent dark stage
    const stage = canvas.closest('.slab-dark') || canvas.parentElement;
    let targetPx = 0, curPx = 0, targetPs = 0, curPs = 0;
    const onMove = (e) => {
      const r = canvas.getBoundingClientRect();
      if (!r.width) return;
      targetPx = ((e.clientX - r.left) / r.width) * 2 - 1;
      const relY = (e.clientY - r.top) / Math.max(1, r.height);
      targetPs = relY > -1.8 && relY <= 1.2 ? 1 : 0.35;
    };
    const onLeave = () => { targetPs = 0; };
    if (stage) {
      stage.addEventListener('pointermove', onMove, { passive: true });
      stage.addEventListener('pointerleave', onLeave, { passive: true });
    }

    let voiceTarget = 0, voiceCur = 0, rippleClock = 0;
    let growStart = performance.now();
    let growDur = calm ? 600 : 820;

    api.setVoice = (amp) => {
      voiceTarget = Math.max(0, Math.min(1, amp));
    };
    api.triggerGrow = () => {
      growStart = performance.now();
    };

    let visible = true;
    let raf = 0;
    let lost = false;
    let t0 = performance.now();
    let prev = t0;

    const render = (now) => {
      raf = 0;
      if (lost || !visible || document.hidden || isReducedMotion()) return;

      const dt = Math.min(0.05, (now - prev) / 1000);
      prev = now;
      const elapsed = (now - t0) / 1000 * (calm ? 0.68 : 1.0);
      const grow = Math.min(1, (now - growStart) / growDur);

      curPx += (targetPx - curPx) * Math.min(1, dt * 8);
      curPs += (targetPs - curPs) * Math.min(1, dt * 5);
      voiceCur += (voiceTarget - voiceCur) * Math.min(1, dt * 11);
      if (voiceCur > 0.01) rippleClock += dt * (0.85 + voiceCur * 0.9);

      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

      gl.useProgram(prog);
      gl.bindVertexArray(vao);
      gl.uniform1f(loc.uTime, elapsed);
      gl.uniform1f(loc.uAspect, aspect);
      gl.uniform1f(loc.uGrow, grow);
      gl.uniform3f(loc.uPointer, curPx, 0, curPs);
      gl.uniform2f(loc.uVoice, voiceCur, rippleClock);
      gl.uniform1f(loc.uVoiceAmp, voiceCur);

      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, (SEGS + 1) * 2, count);
      gl.bindVertexArray(null);

      raf = requestAnimationFrame(render);
    };

    const syncLoop = () => {
      if (!lost && visible && !document.hidden && !isReducedMotion()) {
        if (!raf) {
          prev = performance.now();
          raf = requestAnimationFrame(render);
        }
      } else if (raf) {
        cancelAnimationFrame(raf);
        raf = 0;
      }
    };

    let io = null;
    if ('IntersectionObserver' in window) {
      io = new IntersectionObserver((entries) => {
        visible = entries.some((e) => e.isIntersecting);
        syncLoop();
      }, { threshold: 0.02 });
      io.observe(canvas);
    }
    document.addEventListener('visibilitychange', syncLoop);

    canvas.addEventListener('webglcontextlost', (e) => {
      e.preventDefault();
      lost = true;
      if (bed) bed.classList.remove('webgl-ready');
      syncLoop();
    });
    canvas.addEventListener('webglcontextrestored', () => {
      lost = false;
      if (bed) bed.classList.add('webgl-ready');
      syncLoop();
    });

    if (bed) bed.classList.add('webgl-ready');
    syncLoop();

    api.destroy = () => {
      if (raf) cancelAnimationFrame(raf);
      if (io) io.disconnect();
      document.removeEventListener('visibilitychange', syncLoop);
      window.removeEventListener('resize', resize);
    };
  };

  // Start after first paint so WebGL compilation never delays LCP
  if ('requestIdleCallback' in window) {
    window.requestIdleCallback(boot, { timeout: 300 });
  } else {
    window.setTimeout(boot, 48);
  }

  return api;
}
