import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { LocalServer, Preset, ProviderView, TestReport } from "../api/types";
import { useAiSetup } from "../aiSetup";
import { btnPrimary, btnSecondary, ErrorLine, errMsg, field } from "../drawer/ui";
import { useStore } from "../store";
import { DEFAULT_MODEL, LocalDownload } from "./LocalDownload";

type Cloud = { id: string; blurb: string };

const CLOUD: Cloud[] = [
  { id: "gemini", blurb: "About 500 free requests a day, no card. Outside the EU, UK and Switzerland, Google may use free-tier questions, including folder and file names, to improve its products. Not free in the EU, UK or Switzerland." },
  { id: "openrouter", blurb: "50 free requests a day (1,000 after a one-time $10 top-up), no card. Whether a free model's host keeps your questions depends on the model; OpenRouter's privacy settings can block hosts that do." },
  { id: "ollama-cloud", blurb: "A small free monthly allowance, one request at a time. Ollama says it never logs or trains on your questions." },
  { id: "mistral", blurb: "A free monthly allowance, no card. Mistral may train on free-tier questions unless you opt out in its settings." },
  { id: "cloudflare", blurb: "A free daily allowance of about 300 short requests. Cloudflare doesn't train on your questions. Needs your account ID as well as a token." },
  { id: "zai", blurb: "Two GLM models are free; limits aren't published. Z.ai doesn't use your questions without consent." },
];

const SEND_NOTE = "Cloud choices send your questions, and the file and folder names Hoardini looks up to answer them, to that provider.";

type View = { kind: "choose" } | { kind: "cloud"; preset: Preset; blurb: string } | { kind: "local" } | { kind: "download" };

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

function CloudSetup({ preset, blurb, onBack, onDone }: { preset: Preset; blurb: string; onBack: () => void; onDone: () => void }) {
  const [key, setKey] = useState("");
  const [account, setAccount] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<TestReport | null>(null);
  const needsAccount = preset.base_url.includes("{account_id}");

  const go = async () => {
    setBusy(true);
    setError(null);
    setReport(null);
    try {
      const existing = await routes.providers();
      const base_url = needsAccount ? preset.base_url.replace("{account_id}", encodeURIComponent(account.trim())) : preset.base_url;
      const view = await saveProvider(existing, { name: preset.name, base_url }, preset.default_model, key.trim());
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
          <div className="text-[length:var(--fs-sm)] text-muted">{blurb}</div>
          {preset.key_url && <ExternalLink href={preset.key_url} primary>Get a free key</ExternalLink>}
        </li>
        <li className="space-y-1.5">
          <div className="font-semibold">2. Paste it here</div>
          {needsAccount && <input className={field} placeholder="Account ID" value={account} onChange={(e) => setAccount(e.target.value)} />}
          <input className={field} type="password" autoComplete="new-password" placeholder="API key" value={key} onChange={(e) => setKey(e.target.value)} />
          <div className="text-[length:var(--fs-xs)] text-faint">Stored encrypted on this PC; never shown again.</div>
        </li>
        <li className="space-y-1.5">
          <div className="font-semibold">3. Test it</div>
          <div className="text-[length:var(--fs-xs)] text-faint">
            Uses {preset.default_model}. Hoardini checks the model exists and can call tools.
          </div>
          <button className={btnPrimary} disabled={busy || !key.trim() || (needsAccount && !account.trim())} onClick={() => void go()}>
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
  const [more, setMore] = useState(false);

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
    if (open) {
      setView({ kind: "choose" });
      setMore(false);
    }
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
            <h2 className="font-[family-name:var(--font-display)] text-[19px] font-extrabold">Set up Hoardini's AI</h2>
            <p className="text-muted">Hoardini uses a small AI on this PC to work out what is taking space. Your files and questions never leave it.</p>
            <LocalDownload only={DEFAULT_MODEL} onDone={finish} />
            {!more ? (
              <button className="text-[length:var(--fs-sm)] font-semibold text-brass hover:underline" onClick={() => setMore(true)}>
                Other ways to connect
              </button>
            ) : (
              <ul className="space-y-2">
                {CLOUD.map((c) => {
                  const preset = presets.find((p) => p.id === c.id);
                  if (!preset) return null;
                  return (
                    <li key={c.id}>
                      <button className="w-full rounded-xl border border-line-soft bg-ink-850 p-3 text-left hover:border-brass" onClick={() => setView({ kind: "cloud", preset, blurb: c.blurb })}>
                        <div className="font-bold">{preset.name}</div>
                        <div className="text-[length:var(--fs-sm)] text-muted">{c.blurb}</div>
                      </button>
                    </li>
                  );
                })}
                <li>
                  <button className="w-full rounded-xl border border-line-soft bg-ink-850 p-3 text-left hover:border-brass" onClick={() => setView({ kind: "download" })}>
                    <div className="font-bold">A different model on this PC</div>
                    <div className="text-[length:var(--fs-sm)] text-muted">Qwen3.5 9B (smarter, needs a graphics card), or a model file you already have.</div>
                  </button>
                </li>
                <li>
                  <button className="w-full rounded-xl border border-line-soft bg-ink-850 p-3 text-left hover:border-brass" onClick={() => setView({ kind: "local" })}>
                    <div className="font-bold">LM Studio, Ollama or llama.cpp</div>
                    <div className="text-[length:var(--fs-sm)] text-muted">Uses one that is already running. Nothing leaves this PC.</div>
                  </button>
                </li>
                <li>
                  <button className="w-full rounded-xl border border-line-soft bg-ink-850 p-3 text-left hover:border-brass" onClick={own}>
                    <div className="font-bold">Set up a provider</div>
                    <div className="text-[length:var(--fs-sm)] text-muted">Any OpenAI-compatible service or server, by hand.</div>
                  </button>
                </li>
              </ul>
            )}
            <p className="text-[length:var(--fs-xs)] text-faint">{more && SEND_NOTE}</p>
            <div className="flex justify-end">
              <button className={btnSecondary} onClick={later}>
                Later
              </button>
            </div>
          </>
        )}
        {view.kind === "cloud" && <CloudSetup preset={view.preset} blurb={view.blurb} onBack={() => setView({ kind: "choose" })} onDone={finish} />}
        {view.kind === "local" && <LocalSetup onBack={() => setView({ kind: "choose" })} onDone={finish} />}
        {view.kind === "download" && (
          <div className="space-y-3">
            <h3 className="font-bold">Download a model to this PC</h3>
            <LocalDownload onDone={finish} />
            <button className={btnSecondary} onClick={() => setView({ kind: "choose" })}>
              Back
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
