import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { HistoryPoint } from "../api/types";
import { humanBytes } from "../format";
import { ErrorLine, errMsg } from "./ui";

const W = 520;
const H = 200;
const PAD = { l: 52, r: 12, t: 12, b: 26 };

export function FreeSpaceChart({ points }: { points: HistoryPoint[] }) {
  const xs = points.map((p) => p.finished_ms);
  const ys = points.map((p) => p.free_bytes);
  const x0 = Math.min(...xs);
  const x1 = Math.max(...xs);
  const yMax = Math.max(...ys) * 1.1 || 1;
  const sx = (x: number) => PAD.l + (x1 === x0 ? (W - PAD.l - PAD.r) / 2 : ((x - x0) / (x1 - x0)) * (W - PAD.l - PAD.r));
  const sy = (y: number) => H - PAD.b - (y / yMax) * (H - PAD.t - PAD.b);
  const line = points.map((p, i) => `${i ? "L" : "M"}${sx(p.finished_ms).toFixed(1)} ${sy(p.free_bytes).toFixed(1)}`).join(" ");
  const area = `${line} L${sx(x1).toFixed(1)} ${H - PAD.b} L${sx(x0).toFixed(1)} ${H - PAD.b} Z`;
  const ticks = [0, 0.5, 1].map((t) => t * yMax);
  const fmt = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full" role="img" aria-label="Free space over time">
      {ticks.map((t) => (
        <g key={t}>
          <line x1={PAD.l} x2={W - PAD.r} y1={sy(t)} y2={sy(t)} stroke="var(--color-line-soft)" />
          <text x={PAD.l - 6} y={sy(t) + 4} textAnchor="end" fontSize="10" fill="var(--color-faint)">
            {humanBytes(t, 0)}
          </text>
        </g>
      ))}
      <path d={area} fill="var(--color-brass)" opacity="0.14" />
      <path d={line} fill="none" stroke="var(--color-brass)" strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" />
      {points.map((p) => (
        <circle key={p.finished_ms} cx={sx(p.finished_ms)} cy={sy(p.free_bytes)} r="3" fill="var(--color-ink-900)" stroke="var(--color-brass)" strokeWidth="1.5">
          <title>{`${new Date(p.finished_ms).toLocaleString()}: ${humanBytes(p.free_bytes)} free`}</title>
        </circle>
      ))}
      <text x={PAD.l} y={H - 8} fontSize="10" fill="var(--color-faint)">
        {fmt(x0)}
      </text>
      <text x={W - PAD.r} y={H - 8} fontSize="10" textAnchor="end" fill="var(--color-faint)">
        {fmt(x1)}
      </text>
    </svg>
  );
}

export function HistoryTab() {
  const [points, setPoints] = useState<HistoryPoint[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    routes.history().then((p) => setPoints([...p].sort((a, b) => a.finished_ms - b.finished_ms))).catch((e) => setError(errMsg(e)));
  }, []);
  const first = points?.[0];
  const last = points?.[points.length - 1];
  return (
    <div className="space-y-3">
      <p className="text-muted">Free space after each index refresh.</p>
      <ErrorLine error={error} />
      {points == null && !error && <div className="text-muted pulse-soft">Loading...</div>}
      {points?.length === 0 && <div className="text-muted">No history yet. It builds up as the index refreshes.</div>}
      {points && points.length > 0 && first && last && (
        <>
          <div className="rounded-xl border border-line-soft bg-ink-850 p-2">
            <FreeSpaceChart points={points} />
          </div>
          <div className="text-[length:var(--fs-sm)] text-muted">
            Now {humanBytes(last.free_bytes)} free of {humanBytes(last.total_bytes)}
            {points.length > 1 && (
              <>
                {" "}
                · {last.free_bytes >= first.free_bytes ? "up" : "down"} {humanBytes(Math.abs(last.free_bytes - first.free_bytes))} since {new Date(first.finished_ms).toLocaleDateString()}
              </>
            )}
          </div>
        </>
      )}
    </div>
  );
}
