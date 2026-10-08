import { useCallback, useEffect, useState } from "react";
import { routes } from "../api/client";
import type { BuiltinModel, BuiltinView, Job } from "../api/types";
import { humanBytes } from "../format";
import { useStore } from "../store";
import { btnDanger, btnPrimary, btnSecondary, ErrorLine, errMsg } from "./ui";

const HEADROOM = 512 * 1024 * 1024;
const LIVE = ["queued", "running"];

function License({ name, url }: { name: string; url: string }) {
  return (
    <a className="text-brass underline" href={url} target="_blank" rel="noreferrer">
      {name}
    </a>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-3">
      <span className="text-muted">{label}</span>
      <span className="font-mono">{value}</span>
    </div>
  );
}

function Progress({ job, onCancel }: { job: Job; onCancel: () => void }) {
  const { done_bytes, total_bytes, current } = job.progress;
  const pct = total_bytes > 0 ? Math.min(100, (done_bytes / total_bytes) * 100) : 0;
  return (
    <div className="space-y-1.5">
      <div className="flex justify-between text-[length:var(--fs-sm)]">
        <span className="text-muted">Downloading {current ?? ""}</span>
        <span className="font-mono">
          {humanBytes(done_bytes)} / {humanBytes(total_bytes)}
        </span>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-ink-800" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
        <div className="h-full rounded-full bg-brass transition-[width]" style={{ width: `${pct}%` }} />
      </div>
      <div className="flex justify-end">
        <button className={btnSecondary} onClick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}

// One card for the on-device model: what the download costs, its license, progress, and removal.
export function BuiltinCard({ active, onChange }: { active: boolean; onChange: () => void }) {
  const [view, setView] = useState<BuiltinView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [freed, setFreed] = useState<number | null>(null);
  const jobId = view?.install_job?.id;
  const live = useStore((s) => (jobId ? s.jobs[jobId] : undefined));
  const job = live ?? view?.install_job ?? null;
  const busy = !!job && LIVE.includes(job.status);

  const load = useCallback(() => {
    routes
      .builtin()
      .then(setView)
      .catch((e) => setError(errMsg(e)));
  }, []);
  useEffect(load, [load]);

  const finished = job && !LIVE.includes(job.status) ? job.status : null;
  useEffect(() => {
    if (finished) {
      load();
      onChange();
    }
  }, [finished, load, onChange]);

  if (!view) return null;
  const pick: BuiltinModel | undefined = view.model ?? view.candidates.find((c) => c.id === view.recommended);
  // No candidate passed the eval: the option stays hidden until a better small model does.
  if (!pick || (!view.recommended && !view.installed && !busy)) return null;

  const download = (view.runtime.installed ? 0 : view.runtime.size) + (pick.installed ? 0 : pick.size);
  const roomy = view.free_bytes >= download + HEADROOM;

  const run = async (fn: () => Promise<void>) => {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errMsg(e));
    }
  };

  return (
    <section className="space-y-2.5 rounded-xl border border-line-soft bg-ink-850 p-3">
      <div className="flex items-start justify-between gap-2">
        <div>
          <div className="font-bold">Built-in model</div>
          <p className="text-[length:var(--fs-sm)] text-muted">
            {pick.name} runs on this PC's CPU ({view.threads} threads). No API key, and with web search off nothing leaves the machine.
          </p>
        </div>
        {view.installed && (
          <span className={`shrink-0 rounded-full px-2 py-px text-[length:var(--fs-xs)] font-bold ${view.running ? "bg-ok/10 text-ok" : "bg-ink-800 text-muted"}`}>
            {view.running ? "running" : "idle"}
          </span>
        )}
      </div>

      {!view.installed && (
        <div className="space-y-1 rounded-lg border border-line-soft bg-ink-900 p-2.5 text-[length:var(--fs-sm)]">
          <Row label={`Runtime (llama.cpp ${view.runtime.version})`} value={view.runtime.installed ? "installed" : humanBytes(view.runtime.size)} />
          <Row label={`Model (${pick.name})`} value={pick.installed ? "installed" : humanBytes(pick.size)} />
          <Row label="Free space" value={humanBytes(view.free_bytes)} />
          <div className="pt-1 text-muted">
            License: <License name={pick.license} url={pick.license_url} /> · runtime <License name={view.runtime.license} url={view.runtime.license_url} />. Downloaded from the publishers and checked against pinned hashes.
          </div>
        </div>
      )}

      {busy && job ? (
        <Progress job={job} onCancel={() => void run(async () => void (await routes.cancelJob(job.id)))} />
      ) : view.installed ? (
        <div className="space-y-2">
          <div className="text-[length:var(--fs-sm)] text-muted">
            {humanBytes(pick.size)} on disk · license <License name={pick.license} url={pick.license_url} />
          </div>
          <div className="flex flex-wrap justify-end gap-2">
            {!active && view.provider_id && (
              <button className={btnPrimary} onClick={() => void run(async () => (await routes.activateProvider(view.provider_id!), onChange()))}>
                Use built-in
              </button>
            )}
            {confirm ? (
              <>
                <button
                  className={btnDanger}
                  onClick={() =>
                    void run(async () => {
                      const r = await routes.removeBuiltin();
                      setFreed(r.freed_bytes);
                      setConfirm(false);
                      load();
                      onChange();
                    })
                  }
                >
                  Remove and free {humanBytes(pick.size + view.runtime.size)}
                </button>
                <button className={btnSecondary} onClick={() => setConfirm(false)}>
                  Keep
                </button>
              </>
            ) : (
              <button className={btnSecondary} onClick={() => setConfirm(true)}>
                Remove built-in model
              </button>
            )}
          </div>
        </div>
      ) : (
        <div className="flex items-center justify-between gap-2">
          <span className="text-[length:var(--fs-sm)] text-muted">
            {freed != null
              ? `Removed. Freed ${humanBytes(freed)}.`
              : finished === "failed" && job?.error
                ? `Last try failed: ${job.error}`
                : roomy
                  ? ""
                  : "Not enough free space for this download."}
          </span>
          <button className={btnPrimary} disabled={!roomy} onClick={() => void run(async () => setView({ ...view, install_job: await routes.installBuiltin(pick.id) }))}>
            Download {humanBytes(download)}
          </button>
        </div>
      )}
      <ErrorLine error={error} />
    </section>
  );
}
