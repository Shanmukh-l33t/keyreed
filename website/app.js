const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const nav = document.getElementById("nav");
const menuBtn = document.getElementById("menu-btn");
const mobile = document.getElementById("mobile-nav");
menuBtn.addEventListener("click", () => {
  const open = mobile.hasAttribute("hidden");
  if (open) mobile.removeAttribute("hidden");
  else mobile.setAttribute("hidden", "");
  mobile.classList.toggle("open", open);
  menuBtn.setAttribute("aria-expanded", String(open));
});
mobile.querySelectorAll("a").forEach((a) => a.addEventListener("click", () => {
  mobile.setAttribute("hidden", "");
  mobile.classList.remove("open");
  menuBtn.setAttribute("aria-expanded", "false");
}));
addEventListener("scroll", () => nav.classList.toggle("stuck", scrollY > 8), { passive: true });

const modes = {
  smart: ["Smart Writing", "Cleans grammar and keeps the meaning. Spoken punctuation stays words.", "I checked the server, but I couldn't identify why the site went down."],
  exact: ["Exact Dictation", "Stays closer to the words you said.", "I checked the server but I am not able to find what is the reason for the website going down."],
  pro: ["Professional Writing", "A more formal line. Symbols stay words.", "I checked the server but could not identify the cause of the outage."],
  casual: ["Casual Writing", "A lighter line for chat.", "Checked the server and still can't see why the site went down."],
  tech: ["Technical", "What a focused code editor selects. Symbols stay words.", "The outage is not explained by the server check."],
  term: ["Terminal / Code", "The only mode that turns spoken symbols into characters.", "cd /var/log"]
};
const label = document.getElementById("mode-label");
const copy = document.getElementById("mode-copy");
const sample = document.getElementById("mode-sample");
function setMode(key) {
  document.querySelectorAll(".mode-row button").forEach((b) => {
    const on = b.dataset.mode === key;
    b.classList.toggle("on", on);
    b.setAttribute("aria-selected", String(on));
  });
  const row = modes[key];
  label.textContent = row[0];
  copy.textContent = row[1];
  sample.textContent = row[2];
}
document.querySelectorAll(".mode-row button").forEach((b) => b.addEventListener("click", () => setMode(b.dataset.mode)));
setMode("smart");

const world = document.getElementById("world");
const field = document.getElementById("field-text");
const heroState = document.getElementById("hero-state");
const status = document.getElementById("status");
const hold = document.getElementById("hold");
const wave = document.getElementById("wave");
const wctx = wave.getContext("2d");
let phase = 0;

function drawWave(active) {
  const w = wave.width, h = wave.height;
  wctx.clearRect(0, 0, w, h);
  wctx.beginPath();
  wctx.strokeStyle = "#1f6b4a";
  wctx.lineWidth = 2;
  for (let x = 0; x < w; x++) {
    const n = active ? Math.sin(x * 0.045 + phase) * 16 : Math.sin(x * 0.03) * 2;
    const y = h / 2 + n;
    if (x === 0) wctx.moveTo(x, y); else wctx.lineTo(x, y);
  }
  wctx.stroke();
  if (active && !reduce) { phase += 0.16; requestAnimationFrame(() => drawWave(true)); }
}
drawWave(false);

function setHero(name, text) {
  world.className = "world " + name;
  heroState.textContent = name === "done" ? "Ready" : name[0].toUpperCase() + name.slice(1);
  field.textContent = text;
}
function startHold() {
  hold.setAttribute("aria-pressed", "true");
  status.textContent = "Listening illustration. No microphone is open.";
  setHero("listening", "Listening.");
  drawWave(true);
}
function endHold() {
  if (hold.getAttribute("aria-pressed") !== "true") return;
  hold.setAttribute("aria-pressed", "false");
  status.textContent = "Correcting the sample line.";
  setHero("processing", "Correcting.");
  setTimeout(() => {
    status.textContent = "Ready to paste. This page did not copy anything to your clipboard.";
    setHero("done", "I checked the server but couldn't identify the reason the website is going down.");
    drawWave(false);
  }, reduce ? 0 : 700);
}
hold.addEventListener("pointerdown", (e) => { e.preventDefault(); startHold(); });
hold.addEventListener("pointerup", endHold);
hold.addEventListener("pointerleave", endHold);
hold.addEventListener("keydown", (e) => { if (e.key === " " || e.key === "Enter") { e.preventDefault(); startHold(); } });
hold.addEventListener("keyup", (e) => { if (e.key === " " || e.key === "Enter") endHold(); });
document.getElementById("play").addEventListener("click", () => {
  startHold();
  if (!reduce) setTimeout(endHold, 1200);
  else endHold();
});

const stage = document.getElementById("stage");
if (!reduce) {
  stage.addEventListener("pointermove", (e) => {
    const r = stage.getBoundingClientRect();
    const x = (e.clientX - r.left) / r.width - 0.5;
    const y = (e.clientY - r.top) / r.height - 0.5;
    world.style.transform = "rotateY(" + (x * 14) + "deg) rotateX(" + (-y * 8) + "deg)";
  });
  stage.addEventListener("pointerleave", () => { world.style.transform = ""; });
}

const canvas = document.getElementById("gl");
const gl = canvas.getContext("webgl", { antialias: true, alpha: true });
if (gl) {
  const vs = "attribute vec3 p;attribute vec2 uv;uniform mat4 m;varying vec2 v;void main(){v=uv;gl_Position=m*vec4(p,1.0);}";
  const fs = "precision mediump float;varying vec2 v;uniform sampler2D tex;uniform float shade;void main(){vec4 c=texture2D(tex,v);gl_FragColor=vec4(c.rgb*shade,1.0);}";
  function sh(type, src) { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return s; }
  const prog = gl.createProgram();
  gl.attachShader(prog, sh(gl.VERTEX_SHADER, vs));
  gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(prog);
  gl.useProgram(prog);
  const data = new Float32Array([
    -0.42,-0.05,0.12, 0,1,  0.42,-0.05,0.12, 1,1,  0.48,0.52,0.12, 1,0,
    -0.42,-0.05,0.12, 0,1,  0.48,0.52,0.12, 1,0,  -0.48,0.52,0.12, 0,0,
    -0.5,-0.42,-0.08, 0,1,  0.5,-0.42,-0.08, 1,1,  0.42,-0.05,0.12, 1,0,
    -0.5,-0.42,-0.08, 0,1,  0.42,-0.05,0.12, 1,0,  -0.42,-0.05,0.12, 0,0
  ]);
  const buf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
  const loc = gl.getAttribLocation(prog, "p");
  const uv = gl.getAttribLocation(prog, "uv");
  gl.enableVertexAttribArray(loc);
  gl.enableVertexAttribArray(uv);
  gl.vertexAttribPointer(loc, 3, gl.FLOAT, false, 20, 0);
  gl.vertexAttribPointer(uv, 2, gl.FLOAT, false, 20, 12);
  const mu = gl.getUniformLocation(prog, "m");
  const su = gl.getUniformLocation(prog, "shade");
  const tex = gl.createTexture();
  const img = new Image();
  img.onload = () => {
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, 1);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  };
  img.src = "assets/icon.png";
  gl.clearColor(0, 0, 0, 0);
  let t = 0;
  function frame() {
    const yaw = reduce ? 0.35 : 0.4 + Math.sin(t) * 0.22;
    const pitch = -0.25;
    const c = Math.cos(yaw), s = Math.sin(yaw), cp = Math.cos(pitch), sp = Math.sin(pitch);
    const m = new Float32Array([
      c, sp*s, cp*s, 0,
      0, cp, -sp, 0,
      -s, sp*c, cp*c, 0,
      0, -0.02, 0, 1
    ]);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.uniformMatrix4fv(mu, false, m);
    gl.uniform1f(su, 0.35);
    gl.drawArrays(gl.TRIANGLES, 6, 6);
    gl.uniform1f(su, 1);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    if (!reduce) { t += 0.012; requestAnimationFrame(frame); }
  }
  frame();
}

if (!reduce) {
  const seq = [
    ["idle", "Ready when you are."],
    ["listening", "Listening."],
    ["processing", "Correcting."],
    ["done", "I checked the server but couldn't identify the reason the website is going down."]
  ];
  let i = 0;
  setInterval(() => {
    if (hold.getAttribute("aria-pressed") === "true") return;
    i = (i + 1) % seq.length;
    setHero(seq[i][0], seq[i][1]);
  }, 2800);
}
