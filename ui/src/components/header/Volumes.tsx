import { humanBytes } from "../../format";

export function VolumeBar({ root, total, free }: { root: string; total: number; free: number }) {
  const pct = total > 0 ? free / total : 0;
  const tone = pct < 0.1 ? "bg-err" : pct < 0.2 ? "bg-brass" : "bg-ok";
  return (
    <div className="flex items-center gap-2 text-[length:var(--fs-sm)]" title={`${humanBytes(free)} free of ${humanBytes(total)}`}>
      <span className="font-bold text-paper-dim">{root.replace(/\\$/, "")}</span>
      <div className="h-2 w-24 overflow-hidden rounded-full bg-ink-800" role="meter" aria-valuemin={0} aria-valuemax={total} aria-valuenow={free} aria-label={`${root} free space`}>
        <div className={`h-full rounded-full ${tone}`} style={{ width: `${Math.max(2, pct * 100)}%` }} />
      </div>
      <span className="tabular-nums text-muted">{humanBytes(free, 0)} free</span>
    </div>
  );
}
