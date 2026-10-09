import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { LocalServer, Preset, ProviderView, TestReport } from "../api/types";
import { useAiSetup } from "../aiSetup";
import { btnPrimary, btnSecondary, ErrorLine, errMsg, field } from "../drawer/ui";
import { useStore } from "../store";

type Cloud = { id: string; blurb: string; recommended?: boolean };

const CLOUD: Cloud[] = [
  { id: "cerebras", recommended: true, blurb: "Free to start and very fast. Cerebras sets the limits and they change; read its terms for how it treats your data." },
  { id: "gemini", blurb: "Free and generous. Google uses free-tier prompts, including folder and file names, to improve its products." },
  { id: "openrouter", blurb: "Free models, 50 requests a day (1,000 after a one-time $10 top-up)." },
];

const SEND_NOTE = "Cloud choices send your questions, and the file and folder names Hoardini looks up to answer them, to that provider.";

type View = { kind: "choose" } | { kind: "cloud"; preset: Preset; blurb: string } | { kind: "local" };

function ExternalLink({ href, children, primary }: { href: string; children: string; primary?: boolean }) {
  return (
    <a className={primary ? btnPrimary : btnSecondary} href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  );
}

async function saveProvider(existing: ProviderView[], preset: { name: string; base_url: string }, model: string | null, keyText: string | null) {
  const old = existing.find((p) => p.name === preset.name && p.base_url === preset.base_url);
  let ref = old?.api_key ?? null;
  if (keyText) {
    ref = `secret://provider/${preset.name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}/api_key`;
    await routes.putSecret(ref, keyText);
  }
  const cfg = {
    name: preset.name,
    base_url: preset.base_url,
    model,
    api_key: ref,
    context_window: old?.context_window ?? 32768,
    timeout_s: old?.timeout_s ?? 0,
    max_tool_steps: old?.max_tool_steps ?? 0,
    small: old?.small ?? false,
    redact_paths: old?.redact_paths ?? true,
    params: old?.params ?? {},
    headers: old?.headers ?? {},
    extra_body: old?.extra_body ?? {},
  };
  return old ? routes.updateProvider(old.id, cfg) : routes.addProvider(cfg);
}

function Outcome({ report }: { report: TestReport }) {
  if (!report.ok) {
    return (
      <div className="rounded-xl border border-err/30 bg-err/10 p-2.5 text-[length:var(--fs-sm)] text-err">
        {report.error ?? "The test did not pass."}
        {report.model_found === false && " That model is not offered on this account."}
      </div>
    );
  }
  const tools = report.caps?.tools !== false;
  return (
    <div className="rounded-xl border border-ok/30 bg-ok/10 p-2.5 text-[length:var(--fs-sm)] text-ok">
      Working.{!tools && " This model did not call a test tool, so answers may be limited."}
    </div>
  );
}

function CloudSetup({ preset, onBack, onDone }: { preset: Preset; onBack: () => void; onDone: () => void }) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<TestReport | null>(null);

  const go = async () => {
    setBusy(true);
    setError(null);
    setReport(null);
    try {
      const existing = await routes.providers();
      const view = await saveProvider(existing, preset, preset.default_model, key.trim());
      setKey("");
      const r = await routes.testProvider(view.id);
      setReport(r);
      if (r.ok) {
        await routes.activateProvider(view.id);
        onDone();
      }
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-3">
      <h3 className="font-bold">{preset.name}</h3>
      <ol className="space-y-3 text-[length:var(--fs-base)]">
        <li className="space-y-1.5">
          <div className="font-semibold">1. Get a free key</div>
          {preset.key_url && <ExternalLink href={preset.key_url} primary>Get a free key</ExternalLink>}
        </li>
        <li className="space-y-1.5">
          <div className="font-semibold">2. Paste it here</div>
          <input className={field} type="password" autoComplete="new-password" placeholder="API key" value={key} onChange={(e) => setKey(e.target.value)} />
          <div className="text-[length:var(--fs-xs)] text-faint">Stored encrypted on this PC; never shown again.</div>
        </li>
        <li className="space-y-1.5">
          <div className="font-semibold">3. Test it</div>
          <div className="text-[length:var(--fs-xs)] text-faint">
            Uses {preset.default_model}. Hoardini checks the model exists and can call tools.
          </div>
          <button className={btnPrimary} disabled={busy || !key.trim()} onClick={() => void go()}>
            {busy ? "Testing..." : "Save and test"}
          </button>
        </li>
      </ol>
      <ErrorLine error={error} />
      {report && <Outcome report={report} />}
      <button className={btnSecondary} onClick={onBack}>
        Back
      </button>
    </div>
  );
}

function LocalSetup({ onBack, onDone }: { onBack: () => void; onDone: () => void }) {
  const [servers, setServers] = useState<LocalServer[] | null>(null);
  const [pick, setPick] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<TestReport | null>(null);

  const scan = () => {
    setServers(null);
    routes.localServers().then(setServers).catch(() => setServers([]));
  };
  useEffect(scan, []);

  const use = async (s: LocalServer) => {
    setBusy(true);
    setError(null);
    try {
      const model = pick[s.id] ?? s.models[0] ?? null;
      const view = await saveProvider(await routes.providers(), s, model, null);
      const r = await routes.testProvider(view.id);
      setReport(r);
      if (r.ok) {
        await routes.activateProvider(view.id);
        onDone();
      }
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-3">
      <h3 className="font-bold">A model on this PC</h3>
      {servers === null && <div className="text-muted">Looking for LM Studio, Ollama, or llama.cpp...</div>}
      {servers?.length === 0 && (
        <div className="space-y-2 text-[length:var(--fs-base)]">
          <p>No local model server is running. LM Studio is the easiest way to run one.</p>
          <p className="text-[length:var(--fs-sm)] text-muted">
            You need roughly 8 GB of free memory for a small model (4 to 8 billion parameters) that can use tools. Smaller models answer less reliably. Start the server in LM Studio, then check again.
          </p>
          <div className="flex gap-2">
            <ExternalLink href="https://lmstudio.ai">Get LM Studio</ExternalLink>
            <button className={btnSecondary} onClick={scan}>
              Check again
            </button>
          </div>
        </div>
      )}
      {servers?.map((s) => (
        <div key={s.id} className="space-y-1.5 rounded-xl border border-line-soft bg-ink-850 p-2.5">
          <div className="font-bold">{s.name}</div>
          {s.models.length > 0 ? (
            <select className={field} value={pick[s.id] ?? s.models[0]} onChange={(e) => setPick((p) => ({ ...p, [s.id]: e.target.value }))}>
              {s.models.map((m) => (
                <option key={m}>{m}</option>
              ))}
            </select>
          ) : (
            <div className="text-[length:var(--fs-sm)] text-muted">Running, but no model is loaded yet.</div>
          )}
          <button className={btnPrimary} disabled={busy || s.models.length === 0} onClick={() => void use(s)}>
            {busy ? "Testing..." : "Use this"}
          </button>
        </div>
      ))}
      <ErrorLine error={error} />
      {report && <Outcome report={report} />}
      <div className="text-[length:var(--fs-xs)] text-faint">Everything stays on this PC.</div>
      <button className={btnSecondary} onClick={onBack}>
        Back
      </button>
    </div>
  );
}

export function FirstRun() {
  const open = useAiSetup((s) => s.open);
  const setOpen = useAiSetup((s) => s.setOpen);
  const setDone = useAiSetup((s) => s.setDone);
  const openDrawer = useStore((s) => s.openDrawer);
  const [view, setView] = useState<View>({ kind: "choose" });
  const [presets, setPresets] = useState<Preset[]>([]);

  useEffect(() => {
    routes
      .setup()
      .then((r) => {
        setDone(r.done);
        if (!r.done) setOpen(true);
      })
      .catch(() => {});
    routes.presets().then(setPresets).catch(() => {});
  }, [setDone, setOpen]);

  useEffect(() => {
    if (open) setView({ kind: "choose" });
  }, [open]);

  if (!open) return null;

  const finish = () => {
    void routes.setupDone().catch(() => {});
    setDone(true);
    setOpen(false);
  };
  const later = finish;
  const own = () => {
    finish();
    useAiSetup.getState().requestForm();
    openDrawer("providers");
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div className="absolute inset-0 bg-black/50" aria-hidden />
      <div role="dialog" aria-label="Set up AI" className="relative max-h-full w-full max-w-lg space-y-3 overflow-auto rounded-2xl border border-line bg-ink-900 p-4 shadow-2xl">
        {view.kind === "choose" && (
          <>
            <h2 className="font-[family-name:var(--font-display)] text-[19px] font-extrabold">How should Hoardini think?</h2>
            <p className="text-muted">Pick where the AI runs. Your files are never uploaded; only the questions and the names Hoardini looks up are sent to a cloud provider.</p>
            <ul className="space-y-2">
              {CLOUD.map((c) => {
                const preset = presets.find((p) => p.id === c.id);
                if (!preset) return null;
                return (
                  <li key={c.id}>
                    <button
                      className={`w-full rounded-xl border p-3 text-left hover:border-brass ${c.recommended ? "border-brass/60 bg-brass/5" : "border-line-soft bg-ink-850"}`}
                      onClick={() => setView({ kind: "cloud", preset, blurb: c.blurb })}
                    >
                      <div className="flex items-center gap-2 font-bold">
                        {preset.name}
                        {c.recommended && <span className="rounded-full bg-brass px-2 py-px text-[length:var(--fs-xs)] text-on-accent">Recommended</span>}
                      </div>
                      <div className="text-[length:var(--fs-sm)] text-muted">{c.blurb}</div>
                    </button>
                  </li>
                );
              })}
              <li>
                <button className="w-full rounded-xl border border-line-soft bg-ink-850 p-3 text-left hover:border-brass" onClick={() => setView({ kind: "local" })}>
                  <div className="font-bold">A model on this PC</div>
                  <div className="text-[length:var(--fs-sm)] text-muted">Finds LM Studio, Ollama, or llama.cpp if one is running. Nothing leaves this PC.</div>
                </button>
              </li>
              <li>
                <button className="w-full rounded-xl border border-line-soft bg-ink-850 p-3 text-left hover:border-brass" onClick={own}>
                  <div className="font-bold">Set up a provider</div>
                  <div className="text-[length:var(--fs-sm)] text-muted">Any OpenAI-compatible service or server, by hand.</div>
                </button>
              </li>
            </ul>
            <p className="text-[length:var(--fs-xs)] text-faint">{SEND_NOTE}</p>
            <div className="flex justify-end">
              <button className={btnSecondary} onClick={later}>
                Later
              </button>
            </div>
          </>
        )}
        {view.kind === "cloud" && <CloudSetup preset={view.preset} onBack={() => setView({ kind: "choose" })} onDone={finish} />}
        {view.kind === "local" && <LocalSetup onBack={() => setView({ kind: "choose" })} onDone={finish} />}
      </div>
    </div>
  );
}
