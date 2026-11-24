// Example-only sale: marks rows freed or held and slims the sumo as space comes back.
(() => {
  const TOTAL = 476, START_FREE = 9;
  const says = document.getElementById("sumo-says");
  document.querySelectorAll(".tag").forEach((tag) => {
    tag.setAttribute("aria-pressed", "false");
    const show = () => {
      document.querySelectorAll(".tag").forEach((t) => t.setAttribute("aria-pressed", String(t === tag)));
      says.textContent = tag.dataset.say;
    };
    tag.addEventListener("click", show);
    tag.addEventListener("mouseenter", show);
  });

  const freeEl = document.getElementById("free-gb");
  const note = document.getElementById("tally-note");
  const sumo = document.getElementById("sumo-tally");
  const rows = [...document.querySelectorAll(".rows .row")];

  rows.forEach((row) => {
    const hold = document.createElement("span");
    hold.className = "hold";
    hold.textContent = "HOLD · MINE";
    hold.setAttribute("aria-hidden", "true");
    row.appendChild(hold);
    const go = row.querySelector(".act-go");
    const keep = row.querySelector(".act-keep");
    go.addEventListener("click", () => setRow(row, row.classList.contains("is-gone") ? null : "gone"));
    keep.addEventListener("click", () => setRow(row, row.classList.contains("is-held") ? null : "held"));
  });

  document.querySelector(".clear-all").addEventListener("click", () => {
    document.querySelectorAll(".t-red .row").forEach((r) => { if (!r.classList.contains("is-held")) setRow(r, "gone"); });
  });

  function setRow(row, state) {
    row.classList.toggle("is-gone", state === "gone");
    row.classList.toggle("is-held", state === "held");
    row.querySelector(".act-go").setAttribute("aria-pressed", String(state === "gone"));
    row.querySelector(".act-keep").setAttribute("aria-pressed", String(state === "held"));
    update();
  }

  function update() {
    const freed = rows.filter((r) => r.classList.contains("is-gone")).reduce((n, r) => n + parseFloat(r.dataset.gb), 0);
    const free = START_FREE + freed;
    freeEl.textContent = free % 1 ? free.toFixed(1) : String(free);
    const used = (TOTAL - free) / TOTAL;
    let src, alt, msg;
    if (used > 0.85) { src = freed ? "carry-full" : "worry-full"; alt = "Hoardini, still round: the drive is nearly full."; msg = freed ? "Better. He's still carrying a lot." : "He's carrying a lot. Clear a table and watch him slim down."; }
    else if (used > 0.62) { src = "idle-medium"; alt = "Hoardini, slimmer now: the drive has some room."; msg = "Room to breathe. He's lighter already."; }
    else { src = "tada-slim"; alt = "Hoardini, slim and cheering: the drive has plenty of room."; msg = "Look at him go. That's real room on your drive."; }
    sumo.src = `../assets/mascot/${src}.svg`;
    sumo.alt = alt;
    note.textContent = msg;
  }
})();
