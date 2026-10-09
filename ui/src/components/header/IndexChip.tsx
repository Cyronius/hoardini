import { useState } from "react";
import { routes } from "../../api/client";
import { humanAge } from "../../format";
import { useStore } from "../../store";
import { LeafIcon } from "../icons";

export function IndexChip() {
  const idx = useStore((s) => s.status?.index);
  const [busy, setBusy] = useState(false);
  if (!idx) return null;
  const refreshing = idx.state === "refreshing";
  const pct = idx.progress?.fraction;
  const rescan = () => {
    setBusy(true);
    routes
      .refresh()
      .catch(() => {})
      .finally(() => setBusy(false));
  };
  const age = idx.last_refresh_ms ? humanAge(Date.now() - idx.last_refresh_ms) : "never";
  return (
    <div className="flex items-center gap-2 text-[length:var(--fs-sm)] text-muted">
      {refreshing ? (
        <>
          {idx.mode === "background" && (
            <span className="text-ok" title="Lean: low priority, efficiency cores">
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
        </>
      ) : (
        <>
          <span>Index {age}</span>
          <button
            type="button"
            className="rounded-full border border-line px-2.5 py-0.5 font-semibold hover:text-paper disabled:opacity-40"
            title="Update the index now, at the Fast / Lean speed set in Settings"
            disabled={busy}
            onClick={rescan}
          >
            Rescan now
          </button>
        </>
      )}
    </div>
  );
}
