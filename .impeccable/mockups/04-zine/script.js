document.documentElement.classList.add("js");

const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
const marks = document.querySelectorAll(".hl, .doodle");
if (reduce || !("IntersectionObserver" in window)) {
  marks.forEach((m) => m.classList.add("on"));
} else {
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (e.isIntersecting) { e.target.classList.add("on"); io.unobserve(e.target); }
    }
  }, { rootMargin: "0px 0px -18% 0px", threshold: 0.4 });
  marks.forEach((m) => io.observe(m));
}

const btn = document.getElementById("hold");
const status = document.getElementById("hold-status");
let timer = 0;

function start(e) {
  if (e.type === "keydown" && e.key !== " " && e.key !== "Enter") return;
  if (e.type === "keydown") { e.preventDefault(); if (e.repeat) return; }
  btn.classList.remove("done");
  btn.classList.add("holding");
  status.textContent = "Keep holding…";
  clearTimeout(timer);
  timer = setTimeout(() => {
    btn.classList.remove("holding");
    btn.classList.add("done");
    btn.querySelector(".hold-text").textContent = "Deleted";
    status.textContent = "That's how a real delete feels. Nothing was deleted here.";
    setTimeout(reset, 3200);
  }, 1400);
}
function stop() {
  if (!btn.classList.contains("holding")) return;
  clearTimeout(timer);
  btn.classList.remove("holding");
  status.textContent = "Let go too early, so nothing happens. That's the point.";
}
function reset() {
  btn.classList.remove("done");
  btn.querySelector(".hold-text").textContent = "Hold to delete";
  status.textContent = "";
}

btn.addEventListener("pointerdown", start);
btn.addEventListener("keydown", start);
["pointerup", "pointerleave", "pointercancel"].forEach((t) => btn.addEventListener(t, stop));
btn.addEventListener("keyup", (e) => { if (e.key === " " || e.key === "Enter") stop(); });
btn.addEventListener("contextmenu", (e) => e.preventDefault());
