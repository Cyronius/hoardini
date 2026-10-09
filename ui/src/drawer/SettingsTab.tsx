import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { IndexSpeed, Settings } from "../api/types";
import { LeafIcon } from "../components/icons";
import { humanBytes } from "../format";
import { SavedTick, useAutoSave } from "./autosave";
import { ClearHistorySection } from "./ClearHistory";
import { ErrorLine, errMsg, Section } from "./ui";

const SPEEDS: { id: IndexSpeed; label: string; hint: string }[] = [
  { id: "lean", label: "Lean", hint: "Slower, stays out of your way" },
  { id: "fast", label: "Fast", hint: "Finishes sooner, uses more CPU" },
];

export function SettingsTab() {
  const [s, setS] = useState<Settings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const speed = useAutoSave(async (v: IndexSpeed) => {
    try {
      setS(await routes.putSettings({ index_speed: v }));
    } catch (e) {
      routes.settings().then(setS).catch(() => {});
      throw e;
    }
  });

  useEffect(() => {
    routes
      .settings()
      .then(setS)
      .catch((e) => setLoadError(errMsg(e)));
  }, []);

  if (!s) return <div className="text-muted">{loadError ? <ErrorLine error={loadError} /> : <span className="pulse-soft">Loading...</span>}</div>;

  const choose = (v: IndexSpeed) => {
    if (v === s.index_speed) return;
    setS({ ...s, index_speed: v });
    speed.commit(v);
  };
  const current = SPEEDS.find((o) => o.id === s.index_speed) ?? SPEEDS[0]!;

  return (
    <div className="space-y-5">
      <Section title="Index">
        <div className="flex items-start justify-between gap-3">
          <div className="space-y-1">
            <div className="text-[length:var(--fs-sm)] font-bold text-muted">Refresh speed</div>
            <div className="flex items-center gap-2">
              <div role="radiogroup" aria-label="Refresh speed" className="inline-flex rounded-lg border border-line p-0.5">
                {SPEEDS.map((o) => {
                  const on = o.id === s.index_speed;
                  return (
                    <button
                      key={o.id}
                      type="button"
                      role="radio"
                      aria-checked={on}
                      title={o.hint}
                      onClick={() => choose(o.id)}
                      className={`inline-flex items-center gap-1 rounded-md px-3 py-1 text-[length:var(--fs-sm)] font-semibold transition ${on ? "bg-brass text-on-accent" : "text-muted hover:text-paper"}`}
                    >
                      {o.id === "lean" && <LeafIcon size={12} />}
                      {o.label}
                    </button>
                  );
                })}
              </div>
              <SavedTick status={speed.status} />
            </div>
            <div className="text-[length:var(--fs-xs)] text-faint">{current.hint}. Used at startup, while you have Hoardini open, and by Rescan now.</div>
          </div>
          <div className="shrink-0 pt-0.5 text-right text-[length:var(--fs-sm)]">
            <div className="text-faint">Index size</div>
            <div className="font-bold tabular-nums" title="Read-only">
              {humanBytes(s.index_size_bytes)}
            </div>
          </div>
        </div>
        <ErrorLine error={speed.error} />
      </Section>
      <ClearHistorySection />
    </div>
  );
}
