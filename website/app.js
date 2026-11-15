const modes = {
  smart: "Smart Writing cleans grammar and leaves the meaning. It does not turn spoken punctuation into symbols.",
  exact: "Exact Dictation stays closer to the words you said. Use it when a rewrite would be the wrong edit.",
  pro: "Professional Writing is the tone used for mail and formal notes. Spoken symbols stay words.",
  casual: "Casual Writing is for chat. It is still a correction pass, not a different speech engine.",
  tech: "Technical Mode is what a focused code editor selects. It does not apply Terminal / Code symbol substitution.",
  term: "Terminal / Code is for a terminal. In that mode, \"forward slash\" can become /, and \"underscore\" can become _."
};
const modeCopy = document.getElementById("mode-copy");
function setMode(key) {
  document.querySelectorAll(".mode-row button").forEach((b) => b.classList.toggle("on", b.dataset.mode === key));
  modeCopy.textContent = modes[key];
}
document.querySelectorAll(".mode-row button").forEach((b) => b.addEventListener("click", () => setMode(b.dataset.mode)));
setMode("smart");

const hold = document.getElementById("hold");
const status = document.getElementById("status");
const field = document.getElementById("field-text");
const pill = document.getElementById("pill");
const keycap = document.getElementById("keycap");
const canvas = document.getElementById("wave");
const ctx = canvas.getContext("2d");
let raf = 0;
let phase = 0;
const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

function draw(active) {
  const w = canvas.width;
  const h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  ctx.beginPath();
  ctx.strokeStyle = "#1f6b4a";
  ctx.lineWidth = 2;
  for (let x = 0; x < w; x++) {
    const n = active ? Math.sin(x * 0.05 + phase) * 18 * (0.4 + Math.sin(phase + x * 0.01) * 0.3) : Math.sin(x * 0.04) * 2;
    const y = h / 2 + n;
    if (x === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.stroke();
  if (active && !reduce) {
    phase += 0.18;
    raf = requestAnimationFrame(() => draw(true));
  }
}
draw(false);

function startHold() {
  hold.setAttribute("aria-pressed", "true");
  status.textContent = "Recording illustration. No microphone is open.";
  field.textContent = "Listening.";
  pill.textContent = "Listening";
  keycap.classList.add("down");
  cancelAnimationFrame(raf);
  draw(true);
}
function endHold() {
  if (hold.getAttribute("aria-pressed") !== "true") return;
  hold.setAttribute("aria-pressed", "false");
  cancelAnimationFrame(raf);
  status.textContent = "Correcting the sample line.";
  pill.textContent = "Correcting";
  keycap.classList.remove("down");
  window.setTimeout(() => {
    status.textContent = "Ready to paste. This page did not copy anything to your clipboard.";
    pill.textContent = "Ready";
    field.textContent = "I checked the server but couldn't identify the reason the website is going down.";
    draw(false);
  }, reduce ? 0 : 700);
}
hold.addEventListener("pointerdown", (e) => { e.preventDefault(); startHold(); });
hold.addEventListener("pointerup", endHold);
hold.addEventListener("pointerleave", endHold);
hold.addEventListener("keydown", (e) => { if (e.key === " " || e.key === "Enter") { e.preventDefault(); startHold(); } });
hold.addEventListener("keyup", (e) => { if (e.key === " " || e.key === "Enter") endHold(); });
