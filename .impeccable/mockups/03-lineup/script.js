// Lineup page: his one-line explanations, and the "clear the junk" demo.
(() => {
  const acts = [...document.querySelectorAll(".act")];
  const pa = document.querySelector("[data-pa]");
  const idle = pa.textContent;
  const groupName = { junk: "Junk you won’t miss", keep: "Free space, keep the files", call: "Your call" };
  let pinned = null;

  const say = (act) => {
    acts.forEach((a) => a.classList.toggle("is-on", a === act));
    if (!act) { pa.textContent = idle; return; }
    const gb = act.querySelector(".meta span").textContent;
    const cleared = act.classList.contains("struck") ? " Cleared." : "";
    pa.textContent = `${groupName[act.dataset.group]}, ${gb}. ${act.dataset.say}${cleared}`;
  };

  acts.forEach((a) => {
    a.addEventListener("mouseenter", () => say(a));
    a.addEventListener("mouseleave", () => say(pinned));
    a.addEventListener("focus", () => say(a));
    a.addEventListener("blur", () => say(pinned));
    a.addEventListener("click", () => { pinned = pinned === a ? null : a; say(pinned || a); });
  });

  const btn = document.querySelector("[data-clear]");
  const free = document.querySelector("[data-free]");
  const sumo = document.querySelector("[data-sumo]");
  const bill = document.querySelector(".bill");
  const junk = acts.filter((a) => a.dataset.group === "junk");

  btn.addEventListener("click", () => {
    const on = btn.getAttribute("aria-pressed") !== "true";
    btn.setAttribute("aria-pressed", String(on));
    junk.forEach((a, i) => { a.style.setProperty("--d", on ? `${i * 0.12}s` : "0s"); a.classList.toggle("struck", on); });
    bill.classList.toggle("is-cleared", on);
    free.textContent = on ? "50 GB free" : "9 GB free";
    btn.textContent = on ? "Put it back (it’s an example)" : "Clear the junk · 41 GB";
    sumo.src = on ? "../assets/mascot/tada-full.svg" : "../assets/mascot/worry-full.svg";
    sumo.alt = on ? "Hoardini the sumo, celebrating" : "Hoardini the sumo, worried and very round";
    pa.textContent = on ? "41 GB of junk, cleared. Nothing you’d miss: temp files, caches, Windows leftovers and the Recycle Bin." : idle;
  });
})();
