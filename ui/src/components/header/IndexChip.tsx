import { useEffect, useState } from "react";
import { routes } from "../../api/client";
import { humanAge } from "../../format";
import { useStore } from "../../store";
import { LeafIcon, SpinnerIcon } from "../icons";

const GIVE_UP_MS = 30_000;

export function IndexChip() {
  const idx = useStore((s) => s.status?.index);
  // The route answers at once and the refresh runs on; `since` is the last refresh time at the click, so the button waits for a newer one.
  const [since, setSince] = useState<number | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (since == null) return;
    const t = setTimeout(() => setSince(null), GIVE_UP_MS);
    return () => clearTimeout(t);
  }, [since]);
  if (!idx) return null;
  const refreshing = idx.state === "refreshing";
  const pct = idx.progress?.fraction;
  const busy = since != null && (idx.last_refresh_ms ?? 0) === since;
  const rescan = () => {
    setFailed(false);
    setSince(idx.last_refresh_ms ?? 0);
    routes.refresh().catch(() => {
      setSince(null);
      setFailed(true);
    });
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
            className="inline-flex items-center gap-1.5 rounded-full border border-line px-2.5 py-0.5 font-semibold hover:text-paper disabled:opacity-60"
            title="Update the index now, at the Fast / Lean speed set in Settings"
            disabled={busy}
            aria-busy={busy}
            onClick={rescan}
          >
            {busy && <SpinnerIcon />}
            {busy ? "Scanning…" : "Rescan now"}
          </button>
          {failed && <span className="font-semibold text-err">Rescan failed</span>}
        </>
      )}
    </div>
  );
}
