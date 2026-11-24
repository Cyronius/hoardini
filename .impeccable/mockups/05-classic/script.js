// Example drive: Clear 41 GB strikes the junk, fills the free-space bar, and slims the sumo.
(() => {
  const junk = document.getElementById("junk");
  const clear = document.getElementById("clear");
  const reset = document.getElementById("reset");
  const sumo = document.getElementById("sumo");
  const used = document.getElementById("used");
  const free = document.getElementById("free");
  const head = document.getElementById("appH");
  const note = document.getElementById("doneNote");
  if (!junk || !clear) return;
  junk.querySelectorAll(".rows li").forEach((li, i) => li.style.setProperty("--i", i));
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const start = { free: 9, used: "98.1%", head: head.textContent, src: sumo.src, alt: sumo.alt };

  const count = (from, to, ms, out) => {
    if (still) { out(to); return; }
    const t0 = performance.now();
    const step = (t) => {
      const k = Math.min(1, (t - t0) / ms);
      out(Math.round(from + (to - from) * (1 - Math.pow(1 - k, 4))));
      if (k < 1) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  };

  clear.addEventListener("click", () => {
    clear.disabled = true;
    clear.textContent = "Cleared";
    junk.classList.add("cleared");
    used.style.setProperty("--used", "89.5%");
    count(9, 50, 1100, (n) => { free.textContent = `${n} GB free`; });
    head.textContent = "You can free 38 GB without losing anything.";
    note.textContent = "Done. Freed 41 GB.";
    setTimeout(() => {
      sumo.src = "../assets/mascot/tada-medium.svg";
      sumo.alt = "Hoardini celebrating, a little slimmer now";
      sumo.classList.add("pop");
      setTimeout(() => sumo.classList.remove("pop"), 500);
    }, still ? 0 : 700);
    reset.hidden = false;
  });

  reset.addEventListener("click", () => {
    junk.classList.remove("cleared");
    clear.disabled = false;
    clear.textContent = "Clear 41 GB";
    used.style.setProperty("--used", start.used);
    free.textContent = `${start.free} GB free`;
    head.textContent = start.head;
    note.textContent = "";
    sumo.src = start.src;
    sumo.alt = start.alt;
    reset.hidden = true;
    clear.focus();
  });
})();
