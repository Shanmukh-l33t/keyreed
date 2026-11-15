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

const product = document.getElementById("product");
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
  product.className = "product " + name;
  heroState.textContent = name === "done" ? "Ready" : name[0].toUpperCase() + name.slice(1);
  field.textContent = text;
  document.querySelectorAll(".flow li").forEach((li, n) => {
    const map = { idle: 0, listening: 1, processing: 2, done: 3 };
    li.classList.toggle("on", n === map[name]);
  });
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
