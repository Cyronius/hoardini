import { useState } from "react";
import { routes } from "../../api/client";
import { humanAge } from "../../format";
import { useStore } from "../../store";
import { IconButton } from "../IconButton";
import { LeafIcon, RefreshIcon } from "../icons";

export function IndexChip() {
  const idx = useStore((s) => s.status?.index);
  const [busy, setBusy] = useState(false);
  if (!idx) return null;
  const refreshing = idx.state === "refreshing";
  const pct = idx.progress?.fraction;
  const start = (mode: "background" | "fast") => {
    setBusy(true);
    void routes.refresh(mode).finally(() => setBusy(false));
  };
  const age = idx.last_refresh_ms ? humanAge(Date.now() - idx.last_refresh_ms) : "never";
  return (
    <div className="flex items-center gap-2 text-[length:var(--fs-sm)] text-muted">
      {refreshing ? (
        <>
          {idx.mode === "background" && (
            <span className="text-ok" title="Background mode: low priority, efficiency cores">
              <LeafIcon />
            </span>
          )}
          <span className="text-paper-dim">{idx.mode === "fast" ? "Scanning fast" : "Refreshing"}</span>
          <div className="h-1.5 w-20 overflow-hidden rounded-full bg-ink-800">
            <div
              className={`h-full rounded-full bg-brass transition-all ${pct == null ? "pulse-soft w-1/3" : ""}`}
              style={pct != null ? { width: `${Math.max(3, pct * 100)}%` } : undefined}
            />
          </div>
          <span className="tabular-nums">{pct != null ? `${Math.round(pct * 100)}%` : (idx.progress?.phase ?? "")}</span>
          {idx.mode === "background" && (
            <button
              className="rounded-full border border-brass/40 px-2.5 py-0.5 font-bold text-brass hover:bg-brass/10 disabled:opacity-40"
              disabled={busy}
              onClick={() => start("fast")}
            >
              Scan fast
            </button>
          )}
        </>
      ) : (
        <>
          <span>Index {age}</span>
          <IconButton size="sm" title="Refresh in the background" aria-label="Refresh the index in the background" className="text-muted hover:text-paper" disabled={busy} onClick={() => start("background")}>
            <RefreshIcon />
          </IconButton>
          <button className="rounded-full border border-line px-2.5 py-0.5 font-semibold hover:text-paper disabled:opacity-40" disabled={busy} onClick={() => start("fast")}>
            Scan fast
          </button>
        </>
      )}
    </div>
  );
}
