import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { IndexSpeed, OnlineService, Settings } from "../api/types";
import { LeafIcon } from "../components/icons";
import { humanBytes } from "../format";
import { SavedTick, useAutoSave } from "./autosave";
import { ClearHistorySection } from "./ClearHistory";
import { ErrorLine, errMsg, Section, Toggle } from "./ui";

const SERVICES: { id: OnlineService; label: string; hint: string }[] = [
  { id: "huggingface", label: "Hugging Face", hint: "Model files: looks up the repo and compares its published hash with this PC's copy." },
  { id: "winget", label: "winget", hint: "Installers: asks winget for the package and compares its installer hash." },
];

const SPEEDS: { id: IndexSpeed; label: string; hint: string }[] = [
  { id: "lean", label: "Lean", hint: "Slower, stays out of your way" },
  { id: "fast", label: "Fast", hint: "Finishes sooner, uses more CPU" },
];

// Each reply is the whole settings, so writes go one at a time and the last reply is the last write.
let writes: Promise<unknown> = Promise.resolve();
const putInTurn = (patch: Parameters<typeof routes.putSettings>[0]) => {
  const run = writes.then(() => routes.putSettings(patch));
  writes = run.catch(() => {});
  return run;
};

export function SettingsTab() {
  const [s, setS] = useState<Settings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const speed = useAutoSave(async (v: IndexSpeed) => {
    try {
      setS(await putInTurn({ index_speed: v }));
    } catch (e) {
      routes.settings().then(setS).catch(() => {});
      throw e;
    }
  });

  const online = useAutoSave(async (patch: { service: OnlineService; on: boolean }) => {
    try {
      setS(await putInTurn({ online_checks: { asked: true, services: { [patch.service]: patch.on } } }));
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
      <Section title="Online checks" action={<SavedTick status={online.status} />}>
        <p className="text-[length:var(--fs-xs)] text-faint">
          Confirms that a big file can be downloaded again before it is offered as re-downloadable. Sends file names and package or repo names, never
          file contents or hashes. Checks against your own copies, git, and cloud sync always run on this PC.
        </p>
        <div className="space-y-2">
          {SERVICES.map((svc) => (
            <Toggle
              key={svc.id}
              label={svc.label}
              hint={svc.hint}
              checked={s.online_checks.services[svc.id]}
              onChange={(on) => {
                setS({ ...s, online_checks: { asked: true, services: { ...s.online_checks.services, [svc.id]: on } } });
                online.commit({ service: svc.id, on });
              }}
            />
          ))}
        </div>
        <ErrorLine error={online.error} />
      </Section>
      <ClearHistorySection />
    </div>
  );
}
