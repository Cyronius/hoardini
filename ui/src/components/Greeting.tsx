import { humanBytes } from "../format";
import { useStore } from "../store";
import { letter } from "./header/Volumes";
import { selectedVolume } from "./mascot/girth";

export const starter = "rounded-full border border-brass/40 bg-brass/10 px-3 py-1 text-[length:var(--fs-sm)] font-bold text-brass hover:bg-brass/20";

// The chat's fresh state; the drive's numbers are in the review header (UI-WELCOME-BACK).
export function Starters({ onAsk }: { onAsk: (text: string) => void }) {
  const dupes = useStore((s) => s.summary?.duplicate_hint);
  return (
    <div className="fade-up rounded-2xl border border-line bg-ink-850 p-4">
      <div className="font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold text-paper">Ask Hoardini</div>
      <p className="mt-1 text-paper-dim">Ask what something is, what changed, or where your space went. I propose; you decide.</p>
      <div className="mt-3 flex flex-wrap gap-2">
        <button className={starter} onClick={() => onAsk("Show me the biggest hotspots and what changed since the last refresh.")}>
          Show hotspots
        </button>
        {dupes && (
          <button className={starter} title={`${dupes.count} duplicate ${dupes.count === 1 ? "copy" : "copies"}, ${humanBytes(dupes.bytes)}`} onClick={() => onAsk("Show me the largest duplicate files.")}>
            Show duplicates
          </button>
        )}
        <button className={starter} onClick={() => onAsk("How has my free space changed over time?")}>
          Free space over time
        </button>
      </div>
    </div>
  );
}

// A drive with no finished index yet: the review cannot exist until the first scan ends.
export function Hello() {
  const status = useStore((s) => s.status);
  const vol = selectedVolume(status?.volumes);
  const idx = status?.index;
  const pct = idx?.progress?.fraction;
  const drive = vol ? letter(vol.root) : "this PC";
  return (
    <div className="fade-up rounded-2xl border border-line bg-ink-850 p-5">
      <div className="font-[family-name:var(--font-display)] text-[22px] font-extrabold leading-tight text-paper">Hi, I'm Hoardini.</div>
      <p className="mt-1.5 max-w-prose text-paper-dim">
        I'm measuring {drive} for the first time. When the scan finishes, this page lists what can go and what is big but stays. I propose; you decide.
      </p>
      {idx?.state === "refreshing" && (
        <div className="mt-3 flex items-center gap-2 text-[length:var(--fs-sm)] text-muted">
          <div className="h-1.5 w-40 overflow-hidden rounded-full bg-ink-800">
            <div className={`h-full rounded-full bg-brass transition-all ${pct == null ? "pulse-soft w-1/3" : ""}`} style={pct != null ? { width: `${Math.max(3, pct * 100)}%` } : undefined} />
          </div>
          <span className="tabular-nums">{pct != null ? `${Math.round(pct * 100)}%` : (idx.progress?.phase ?? "scanning")}</span>
        </div>
      )}
    </div>
  );
}
