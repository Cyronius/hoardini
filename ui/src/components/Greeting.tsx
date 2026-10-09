import { fileName, humanAge, humanBytes } from "../format";
import { useStore } from "../store";
import { selectedVolume } from "./mascot/girth";

const starter = "rounded-full border border-brass/40 bg-brass/10 px-3 py-1 text-[length:var(--fs-sm)] font-bold text-brass hover:bg-brass/20";

export function Greeting({ onAsk }: { onAsk: (text: string) => void }) {
  const summary = useStore((s) => s.summary);
  const status = useStore((s) => s.status);
  const vol = selectedVolume(status?.volumes);
  const refreshing = status?.index.state === "refreshing";
  const pct = status?.index.progress?.fraction;

  if (!summary) {
    return (
      <div className="fade-up rounded-2xl border border-line bg-ink-850 p-4 text-muted">
        <div className="font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold text-paper">Hi, I'm Hoardini.</div>
        <p className="mt-1">Ask me what is taking up space on this PC. I propose; you decide.</p>
      </div>
    );
  }

  const delta = summary.free_delta_bytes;
  return (
    <div className="fade-up rounded-2xl border border-line bg-ink-850 p-4">
      <div className="font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold text-paper">
        Index from {humanAge(summary.age_ms)}
        {refreshing && pct != null ? ` · refresh ${Math.round(pct * 100)}%` : ""}
      </div>
      <div className="mt-1.5 space-y-1 text-paper-dim">
        {vol && (delta !== 0 || summary.movers.length > 0) && (
          <p>
            Since then, {vol.root.replace(/\\$/, "")} {delta < 0 ? "lost" : "gained"} {humanBytes(Math.abs(delta))} ({humanBytes(vol.free_bytes)} free).
          </p>
        )}
        {vol && delta === 0 && summary.movers.length === 0 && (
          <p>
            {vol.root.replace(/\\$/, "")} has {humanBytes(vol.free_bytes)} free.
          </p>
        )}
        {summary.movers.length > 0 && (
          <p>
            Biggest movers:{" "}
            {summary.movers.slice(0, 3).map((m, i) => (
              <span key={m.path}>
                {i > 0 && ", "}
                <span className="font-semibold" title={m.path}>
                  {m.path.length > 28 ? fileName(m.path) : m.path}
                </span>{" "}
                <span className={m.delta_bytes >= 0 ? "text-err" : "text-ok"}>
                  {m.delta_bytes >= 0 ? "+" : "-"}
                  {humanBytes(Math.abs(m.delta_bytes))}
                </span>
              </span>
            ))}
            .
          </p>
        )}
        {summary.duplicate_hint && (
          <p>
            {summary.duplicate_hint.count} duplicate {summary.duplicate_hint.count === 1 ? "copy" : "copies"} wasting {humanBytes(summary.duplicate_hint.bytes)}.
          </p>
        )}
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        <button className={starter} onClick={() => onAsk("Show me the biggest hotspots and what changed since the last refresh.")}>
          Show hotspots
        </button>
        {summary.duplicate_hint && (
          <button className={starter} onClick={() => onAsk("Show me the largest duplicate files.")}>
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
