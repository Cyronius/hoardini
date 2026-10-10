import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { RarelyUsedApp } from "../api/types";
import { RevealButton } from "../components/ItemTree";
import { humanAge, humanBytes } from "../format";
import { SectionHead } from "./LargeAreas";

function lastRun(a: RarelyUsedApp): string {
  if (a.last_run_ms == null) return "never run";
  return `last run ${humanAge(Date.now() - a.last_run_ms)}`;
}

function AppRow({ app }: { app: RarelyUsedApp }) {
  const [step, setStep] = useState<"idle" | "confirm" | "busy" | "started">("idle");
  const [err, setErr] = useState<string | null>(null);
  const start = async () => {
    setStep("busy");
    setErr(null);
    try {
      await routes.uninstallApp(app.id);
      setStep("started");
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setStep("confirm");
    }
  };
  return (
    <li className="py-2.5" data-app={app.id}>
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <span className="min-w-0 truncate font-bold text-paper" title={app.name}>
              {app.name}
            </span>
            {app.source === "steam" && <span className="shrink-0 rounded-full border border-line px-2 py-px text-[length:var(--fs-xs)] font-semibold text-muted">Steam</span>}
          </div>
          <div className="flex items-center gap-1 text-[length:var(--fs-xs)] text-faint">
            <span className="min-w-0 truncate" title={app.last_run_from ?? app.path}>
              {[app.publisher, lastRun(app)].filter(Boolean).join(" · ")}
            </span>
            <RevealButton path={app.path} />
          </div>
        </div>
        <span className="shrink-0 font-bold tabular-nums text-paper">{humanBytes(app.size)}</span>
        {step === "idle" && (
          <button
            type="button"
            className="shrink-0 rounded-full border border-line px-3 py-1 text-[length:var(--fs-sm)] font-bold text-paper-dim hover:border-brass/60 hover:text-brass"
            onClick={() => setStep("confirm")}
          >
            Uninstall…
          </button>
        )}
      </div>
      {(step === "confirm" || step === "busy") && (
        <div className="fade-up mt-2 rounded-xl border border-line bg-ink-900 p-2.5" role="group" aria-label={`Uninstall ${app.name}`}>
          <p className="text-[length:var(--fs-sm)] text-paper-dim">
            Open {app.name}'s own uninstaller? It asks you to confirm{app.source === "steam" ? " in Steam" : ", and may ask for administrator permission"}. Hoardini deletes nothing itself.
          </p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <button type="button" disabled={step === "busy"} className="rounded-lg bg-brass px-3 py-1 font-bold text-on-accent hover:opacity-90 disabled:opacity-40" onClick={() => void start()}>
              Open the uninstaller
            </button>
            <button type="button" autoFocus className="rounded-lg border border-line px-3 py-1 font-semibold text-muted hover:text-paper" onClick={() => setStep("idle")}>
              Cancel
            </button>
            {err && <span className="text-[length:var(--fs-sm)] text-err">{err}</span>}
          </div>
        </div>
      )}
      {step === "started" && (
        <p className="mt-1 text-[length:var(--fs-sm)] font-semibold text-ok" role="status">
          The uninstaller is open. The next review shows what it freed.
        </p>
      )}
    </li>
  );
}

// The review response carries the list (REVIEW-TRIGGER `apps`); older servers are asked separately.
export function RarelyUsedApps({ given }: { given?: RarelyUsedApp[] }) {
  const [fetched, setFetched] = useState<RarelyUsedApp[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const has = given != null;
  useEffect(() => {
    if (has) return;
    let live = true;
    routes
      .rarelyUsedApps()
      .then((a) => live && setFetched(a))
      .catch((e) => live && setErr(e instanceof Error ? e.message : String(e)));
    return () => {
      live = false;
    };
  }, [has]);
  const apps = given ?? fetched;
  if (err && !has) return <p className="text-[length:var(--fs-sm)] text-muted">Could not list your apps: {err}</p>;
  if (!apps?.length) return null;
  const total = apps.reduce((a, x) => a + x.size, 0);
  return (
    <section aria-label="Apps you rarely use">
      <SectionHead title="Apps you rarely use" count={apps.length}>
        {humanBytes(total)} in apps and games of 2 GB or more that have not run for 90 days. Each uninstalls through its own uninstaller.
      </SectionHead>
      <ul className="divide-y divide-line-soft rounded-2xl border border-line bg-ink-850 px-3">
        {apps.map((a) => (
          <AppRow key={a.id} app={a} />
        ))}
      </ul>
    </section>
  );
}
