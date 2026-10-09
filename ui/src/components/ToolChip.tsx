import { useEffect, useState } from "react";
import { routes } from "../api/client";
import { friendlyToolName, humanElapsed } from "../format";
import { useStore } from "../store";
import type { HistoryPoint } from "../api/types";
import { FreeSpaceChart } from "./FreeSpaceChart";
import { SpinnerIcon, ToolIcon } from "./icons";

const MIN_CHART_POINTS = 3;
// Older results carry no drive; those charts follow the selected drive.
export const RANGE_RE = /^range_ms: (\d+)\.\.(\d+)(?: (.+?))?\s*$/m;

export type ChartRange = { since: number; until: number; volume?: string };

export function rangeFromText(result: string): ChartRange | null {
  const m = RANGE_RE.exec(result);
  return m ? { since: Number(m[1]), until: Number(m[2]), volume: m[3] } : null;
}

export function ChangesChart({ range }: { range: ChartRange | null }) {
  const [points, setPoints] = useState<HistoryPoint[]>([]);
  const since = range?.since ?? null;
  const until = range?.until ?? null;
  const volume = range?.volume;
  useEffect(() => {
    if (since == null || until == null) return;
    let live = true;
    routes
      .history(since, until, volume)
      .then((p) => live && setPoints(p))
      .catch(() => live && setPoints([]));
    return () => {
      live = false;
    };
  }, [since, until, volume]);
  if (points.length < MIN_CHART_POINTS) return null;
  return (
    <div className="mt-1 rounded-xl border border-line-soft bg-ink-850/70 px-2.5 pb-1 pt-2">
      <div className="mb-1 text-[length:var(--fs-xs)] font-bold text-faint">Free space over time</div>
      <FreeSpaceChart points={points} />
    </div>
  );
}

function argSummary(args: Record<string, unknown>): string {
  const first = Object.values(args).find((v) => typeof v === "string" && v.trim());
  if (typeof first === "string") return first.replace(/\s+/g, " ").trim().slice(0, 80);
  const arr = Object.values(args).find(Array.isArray) as unknown[] | undefined;
  return arr ? arr.map(String).join(", ").slice(0, 80) : "";
}

export function ToolChip({ id, name, args, result }: { id: string; name: string; args: Record<string, unknown>; result: string }) {
  const times = useStore((s) => s.toolTimes[id]);
  const noted = useStore((s) => s.noted[id]);
  const [undone, setUndone] = useState(false);
  const [pinned, setPinned] = useState<boolean | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const done = result !== "";
  const elapsed = times?.end ? humanElapsed(times.end - times.start) : null;

  const undo = async () => {
    if (!noted) return;
    try {
      await routes.deleteMemory(noted.memoryId);
      setUndone(true);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  const isPinned = pinned ?? noted?.pinned ?? false;
  const togglePin = async () => {
    if (!noted) return;
    try {
      await routes.updateMemory(noted.memoryId, { pinned: !isPinned });
      setPinned(!isPinned);
      setErr(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  if (name === "remember" && noted) {
    return (
      <div className="my-1 flex items-center gap-2 rounded-full border border-line-soft bg-ink-850 py-0.5 pl-2.5 pr-1 text-[length:var(--fs-sm)]">
        <span className="font-bold text-brass">{undone ? "forgotten" : isPinned ? "noted · pinned" : "noted"}</span>
        <span className={`min-w-0 flex-1 truncate ${undone ? "text-faint line-through" : "text-muted"}`} title={noted.text}>
          {noted.text}
        </span>
        {!undone && (
          <button
            className="rounded-full px-2 py-0.5 font-bold text-brass hover:bg-ink-800"
            onClick={() => void togglePin()}
            aria-pressed={isPinned}
            title={isPinned ? "Unpin: let this note fall off when memory is full" : "Pin: keep this note even when memory is full"}
          >
            {isPinned ? "Unpin" : "Pin"}
          </button>
        )}
        {!undone && (
          <button className="rounded-full px-2 py-0.5 font-bold text-brass hover:bg-ink-800" onClick={() => void undo()} title="Undo: forget this note" aria-label="Undo remember">
            Undo
          </button>
        )}
        {err && <span className="text-err">{err}</span>}
      </div>
    );
  }

  const summary = argSummary(args);
  const shown = name === "changes" ? result.replace(RANGE_RE, "").trimEnd() : result;
  return (
    <>
    {name === "changes" && done && <ChangesChart range={rangeFromText(result)} />}
    <details className="group my-1 rounded-xl border border-line-soft bg-ink-850/70">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-2.5 py-1 text-[length:var(--fs-sm)] text-muted hover:text-paper">
        {done ? <ToolIcon /> : <SpinnerIcon />}
        <span className="shrink-0 font-bold text-paper-dim">{friendlyToolName(name)}</span>
        {summary && <span className="min-w-0 truncate text-faint">{summary}</span>}
        {elapsed && <span className="ml-auto shrink-0 tabular-nums text-faint">{elapsed}</span>}
        <span className={`${elapsed ? "" : "ml-auto"} shrink-0 text-faint transition-transform group-open:rotate-90`}>›</span>
      </summary>
      <div className="space-y-1.5 border-t border-line-soft px-2.5 py-1.5">
        <pre className="overflow-x-auto rounded-lg bg-ink-900 p-2 font-mono text-[length:var(--fs-xs)] text-paper-dim">{JSON.stringify(args, null, 2)}</pre>
        {shown && (
          <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-ink-900 p-2 font-mono text-[length:var(--fs-xs)] text-paper-dim">{shown}</pre>
        )}
      </div>
    </details>
    </>
  );
}
