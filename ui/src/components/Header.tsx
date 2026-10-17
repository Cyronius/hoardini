import { useEffect, useState } from "react";
import { humanAge } from "../format";
import { useReview } from "../review/state";
import { useStore } from "../store";
import { DriveBar } from "./header/Volumes";
import { IconButton } from "./IconButton";
import { GearIcon, RefreshIcon, SpinnerIcon } from "./icons";
import { Mascot, type MascotState } from "./Mascot";
import { selectedVolume, usedFraction } from "./mascot/girth";

const WORRIED_AT = 0.9;

function useNow(ms: number) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [ms]);
  return now;
}

// His pose is the app's status line (UI-MASCOT-FULLNESS): scanning, thinking, carrying, done, full or all clean.
function useMascotState(): MascotState {
  const tadaUntil = useStore((s) => s.tadaUntil);
  const holding = useStore((s) => s.holding);
  const chatBusy = useStore((s) => s.chatBusy);
  const runs = useStore((s) => s.status?.running_runs ?? 0);
  const refreshing = useStore((s) => s.status?.index.state === "refreshing");
  const building = useStore((s) => !!s.status?.review_building);
  const clean = useStore((s) => s.homeClean);
  const used = useStore((s) => usedFraction(selectedVolume(s.status?.volumes)));
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const left = tadaUntil - Date.now();
    if (left <= 0) return;
    setNow(Date.now());
    const t = setTimeout(() => setNow(Date.now()), left + 20);
    return () => clearTimeout(t);
  }, [tadaUntil]);
  if (tadaUntil > now) return "tada";
  if (holding) return "carry";
  if (chatBusy || runs > 0) return "think";
  if (refreshing || building) return "look";
  if (used != null && used >= WORRIED_AT) return "worry";
  if (clean) return "nap";
  return "idle";
}

export function Header() {
  const status = useStore((s) => s.status);
  const connected = useStore((s) => s.connected);
  const openDrawer = useStore((s) => s.openDrawer);
  const builtAt = useReview((s) => s.main?.review.created_ms);
  const running = useReview((s) => s.running);
  const mood = useMascotState();
  const now = useNow(30_000);
  const idx = status?.index;
  const refreshing = idx?.state === "refreshing";
  const pct = idx?.progress?.fraction;
  return (
    <header className="border-b border-line-soft bg-ink-950">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-5 py-2">
        <Mascot state={mood} volume={selectedVolume(status?.volumes)} className="h-14 w-14 shrink-0" />
        <DriveBar />
        <div className="ml-auto flex items-center gap-2 text-[length:var(--fs-sm)] text-muted">
          {!connected && <span className="font-bold text-err">offline</span>}
          <span className="tabular-nums" role="status">
            {refreshing ? `Looking${pct != null ? ` ${Math.round(pct * 100)}%` : "…"}` : builtAt ? `Checked ${humanAge(now - builtAt)}` : ""}
          </span>
          <IconButton title="Look again now" aria-label="Look again now" className="text-muted hover:text-paper" disabled={running} onClick={() => void useReview.getState().runNow()}>
            {running ? <SpinnerIcon size={16} /> : <RefreshIcon />}
          </IconButton>
          <IconButton title="Settings" aria-label="Open settings" className="text-muted hover:text-paper" onClick={() => openDrawer("main")}>
            <GearIcon />
          </IconButton>
        </div>
      </div>
      {refreshing && (
        <div className="h-0.5 bg-ink-800" aria-hidden="true">
          <div className={`h-full bg-brass transition-all ${pct == null ? "pulse-soft w-1/3" : ""}`} style={pct != null ? { width: `${Math.max(2, pct * 100)}%` } : undefined} />
        </div>
      )}
    </header>
  );
}
