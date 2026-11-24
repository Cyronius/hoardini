// Example demo: emptying the boxes slims the sumo, like the app does as the drive frees up.
(() => {
  const TOTAL = 476;
  const steps = [
    { free: 9, sumo: "worry-full", alt: "Hoardini, big and worried: the drive is almost full" },
    { free: 50, sumo: "idle-medium", alt: "Hoardini, a little slimmer after the junk box is cleared" },
    { free: 88, sumo: "tada-slim", alt: "Hoardini, slim and cheering after both boxes are emptied" },
  ];
  const sumo = document.getElementById("demo-sumo");
  const fill = document.getElementById("gauge-fill");
  const free = document.getElementById("gauge-free");
  const buttons = document.querySelectorAll(".demo-actions [data-step]");
  if (!sumo || !fill || !free) return;
  steps.forEach((s) => { const i = new Image(); i.src = `../assets/mascot/${s.sumo}.svg`; });

  const go = (n) => {
    const s = steps[n];
    fill.style.width = `${(((TOTAL - s.free) / TOTAL) * 100).toFixed(1)}%`;
    free.textContent = `${s.free} GB`;
    sumo.src = `../assets/mascot/${s.sumo}.svg`;
    sumo.alt = s.alt;
    sumo.classList.remove("pop"); void sumo.offsetWidth; sumo.classList.add("pop");
    buttons.forEach((b) => {
      const k = Number(b.dataset.step);
      if (k === 0) b.hidden = n === 0;
      else b.disabled = k !== n + 1;
    });
  };
  buttons.forEach((b) => b.addEventListener("click", () => go(Number(b.dataset.step))));
})();
