import { useCallback, useEffect, useRef, useState } from "react";
import { routes } from "../api/client";
import type { BuiltinModel, BuiltinView, Job } from "../api/types";
import { Progress } from "../drawer/BuiltinCard";
import { btnPrimary, btnSecondary, ErrorLine, errMsg, field } from "../drawer/ui";
import { humanBytes } from "../format";
import { useStore } from "../store";

const HEADROOM = 512 * 1024 * 1024;
const LIVE = ["queued", "running"];

const BLURB: Record<string, string> = {
  "qwen3.5-9b": "The smartest of the two. Needs a graphics card (or integrated graphics with about 7 GB to spare); on a CPU alone it is too slow to use.",
  "granite-4.1-3b": "Small and quick to download. Runs on the CPU with about 4 GB of memory, so any recent PC can use it. Nothing leaves this PC.",
};

const downloadSize = (c: BuiltinModel) => (c.runtime.installed ? 0 : c.runtime.size) + (c.installed ? 0 : c.size);

// First-run and the Settings card share this: two downloads from Hugging Face, or a model file the user points at.
export const DEFAULT_MODEL = "granite-4.1-3b";

// `only` shows that one model alone, as the first-run default.
export function LocalDownload({ onDone, only }: { onDone: () => void; only?: string }) {
  const [view, setView] = useState<BuiltinView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [path, setPath] = useState("");
  const jobId = view?.install_job?.id;
  const live = useStore((s) => (jobId ? s.jobs[jobId] : undefined));
  const job: Job | null = live ?? view?.install_job ?? null;
  const busy = !!job && LIVE.includes(job.status);
  const finished = job && !LIVE.includes(job.status) ? job.status : null;

  const load = useCallback(() => {
    routes
      .builtin()
      .then(setView)
      .catch((e) => setError(errMsg(e)));
  }, []);
  useEffect(load, [load]);

  const handled = useRef<string | null>(null);
  useEffect(() => {
    if (finished !== "done" || !jobId || handled.current === jobId) return;
    handled.current = jobId;
    routes
      .builtin()
      .then(async (v) => {
        setView(v);
        if (v.provider_id) await routes.activateProvider(v.provider_id);
        onDone();
      })
      .catch((e) => setError(errMsg(e)));
  }, [finished, jobId, onDone]);

  if (!view) return <ErrorLine error={error} />;

  const start = async (id: string) => {
    setError(null);
    try {
      setView({ ...view, install_job: await routes.installBuiltin(id) });
    } catch (e) {
      setError(errMsg(e));
    }
  };
  const browse = async () => {
    setError(null);
    try {
      const r = await routes.pickModel();
      if (r.path) setPath(r.path);
    } catch (e) {
      setError(errMsg(e));
    }
  };

  return (
    <div className="space-y-3">
      {!only && (
        <p className="text-[length:var(--fs-sm)] text-muted">
          Downloaded from Hugging Face and checked against pinned hashes. Both are Apache-2.0. Everything runs on this PC.
        </p>
      )}
      {busy && job ? (
        <Progress job={job} onCancel={() => void routes.cancelJob(job.id).catch((e) => setError(errMsg(e)))} />
      ) : (
        <ul className="space-y-2">
          {view.candidates.filter((c) => !only || c.id === only).map((c) => {
            const size = downloadSize(c);
            const roomy = view.free_bytes >= size + HEADROOM;
            return (
              <li key={c.id} className={`space-y-1.5 rounded-xl border p-3 ${only ? "border-brass/60 bg-brass/5" : "border-line-soft bg-ink-850"}`}>
                <div className="flex items-baseline justify-between gap-2">
                  <span className="flex items-center gap-2 font-bold">
                    {c.name}
                    {only && <span className="rounded-full bg-brass px-2 py-px text-[length:var(--fs-xs)] text-on-accent">Recommended</span>}
                  </span>
                  <span className="font-mono text-[length:var(--fs-sm)] text-muted">{size > 0 ? humanBytes(size) : "downloaded"}</span>
                </div>
                <div className="text-[length:var(--fs-sm)] text-muted">{BLURB[c.id]}</div>
                <div className="flex items-center justify-end gap-2">
                  {!roomy && <span className="text-[length:var(--fs-xs)] text-err">Not enough free space.</span>}
                  <button className={btnPrimary} disabled={!roomy} onClick={() => void start(c.id)}>
                    {c.installed && c.runtime.installed ? "Use it" : "Download"}
                  </button>
                </div>
              </li>
            );
          })}
          {!only && (
          <li className="space-y-1.5 rounded-xl border border-line-soft bg-ink-850 p-3">
            <div className="font-bold">A model file on this PC</div>
            <div className="text-[length:var(--fs-sm)] text-muted">
              Point at a .gguf file. It runs on the GPU if there is one, else the CPU. {view.custom_runtime.installed ? "" : `${humanBytes(view.custom_runtime.size)} for the runtime.`}
            </div>
            <div className="flex gap-2">
              <input className={field} placeholder="C:\models\my-model.gguf" value={path} onChange={(e) => setPath(e.target.value)} />
              <button className={btnSecondary} onClick={() => void browse()}>
                Browse
              </button>
            </div>
            <div className="flex justify-end">
              <button className={btnPrimary} disabled={!path.trim()} onClick={() => void start(`file:${path.trim()}`)}>
                Use this file
              </button>
            </div>
          </li>
          )}
        </ul>
      )}
      {finished === "failed" && job?.error && <div className="text-[length:var(--fs-sm)] text-err">Last try failed: {job.error}</div>}
      <ErrorLine error={error} />
    </div>
  );
}
