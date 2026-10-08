import { useCallback, useEffect, useState } from "react";
import { routes } from "../api/client";
import { DEFAULT_AGENT_MAX_STEPS, DEFAULT_AI_TIMEOUT_SECONDS } from "../agent/aiTimeouts";
import type { Preset, ProviderConfig, ProviderView, TestReport } from "../api/types";
import { EditIcon, PlusIcon, TrashIcon } from "../components/icons";
import { IconButton } from "../components/IconButton";
import { btnDanger, btnPrimary, btnSecondary, ErrorLine, errMsg, field, Field, Toggle } from "./ui";

type Form = {
  id?: string;
  name: string;
  base_url: string;
  model: string;
  keyText: string;
  hasKey: boolean;
  context_window: string;
  timeout_s: string;
  max_tool_steps: string;
  small: boolean;
  redact_paths: boolean;
  params: string;
  headers: string;
  extra_body: string;
  apiKeyRef: string | null;
};

const blank = (): Form => ({
  name: "",
  base_url: "",
  model: "",
  keyText: "",
  hasKey: false,
  context_window: "32768",
  timeout_s: "",
  max_tool_steps: "",
  small: false,
  redact_paths: true,
  params: "{}",
  headers: "{}",
  extra_body: "{}",
  apiKeyRef: null,
});

const fromView = (p: ProviderView): Form => ({
  id: p.id,
  name: p.name,
  base_url: p.base_url,
  model: p.model ?? "",
  keyText: "",
  hasKey: p.has_key,
  context_window: String(p.context_window),
  timeout_s: p.timeout_s ? String(p.timeout_s) : "",
  max_tool_steps: p.max_tool_steps ? String(p.max_tool_steps) : "",
  small: p.small,
  redact_paths: p.redact_paths,
  params: JSON.stringify(p.params ?? {}, null, 2),
  headers: JSON.stringify(p.headers ?? {}, null, 2),
  extra_body: JSON.stringify(p.extra_body ?? {}, null, 2),
  apiKeyRef: p.api_key,
});

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "provider";

function parseObject(text: string, label: string): Record<string, unknown> {
  if (!text.trim()) return {};
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch {
    throw new Error(`${label}: invalid JSON`);
  }
  if (v === null || typeof v !== "object" || Array.isArray(v)) throw new Error(`${label}: must be a JSON object`);
  return v as Record<string, unknown>;
}

function posInt(text: string, fallback: number): number {
  const n = Number(text.trim());
  return text.trim() !== "" && Number.isFinite(n) && n > 0 ? Math.floor(n) : fallback;
}

export function toConfig(f: Form, keyRef: string | null): ProviderConfig {
  const headers = parseObject(f.headers, "Headers") as Record<string, string>;
  return {
    id: f.id,
    name: f.name.trim(),
    base_url: f.base_url.trim(),
    model: f.model.trim() || null,
    api_key: keyRef,
    context_window: posInt(f.context_window, 32768),
    timeout_s: posInt(f.timeout_s, 0),
    max_tool_steps: posInt(f.max_tool_steps, 0),
    small: f.small,
    redact_paths: f.redact_paths,
    params: parseObject(f.params, "Parameters") as ProviderConfig["params"],
    headers,
    extra_body: parseObject(f.extra_body, "Extra body"),
  };
}

function Caps({ report }: { report: TestReport }) {
  const c = report.caps;
  if (!c) return null;
  const items: [string, boolean][] = [
    ["tools", c.tools],
    ["streaming", c.streaming],
    ["streamed tool calls", c.streamed_tool_calls],
    ["tool choice", c.tool_choice],
  ];
  return (
    <div className="flex flex-wrap gap-1.5">
      {items.map(([n, ok]) => (
        <span key={n} className={`rounded-full border px-2 py-px text-[length:var(--fs-xs)] font-bold ${ok ? "border-ok/30 bg-ok/10 text-ok" : "border-line bg-ink-800 text-faint line-through"}`}>
          {n}
        </span>
      ))}
    </div>
  );
}

function Report({ report, onPickModel }: { report: TestReport; onPickModel: (m: string) => void }) {
  return (
    <div className={`space-y-1.5 rounded-xl border p-2.5 text-[length:var(--fs-sm)] ${report.ok ? "border-ok/30 bg-ok/10" : "border-err/30 bg-err/10"}`}>
      <div className={`font-bold ${report.ok ? "text-ok" : "text-err"}`}>
        {report.ok ? "Connected" : report.model_found === false ? "Connected, but the model is wrong" : "Could not connect"}
        {report.latency_ms != null && <span className="font-normal text-muted"> · {report.latency_ms} ms</span>}
      </div>
      {/* For an unknown model the line below says the same with clickable suggestions. */}
      {report.error && report.model_found !== false && <div className="text-err">{report.error}</div>}
      {report.model_found === false && (
        <div className="text-brass">
          That model is not on the server; servers like LM Studio would quietly answer with a different one.
          {report.suggestions.length > 0 && (
            <>
              {" "}
              Did you mean{" "}
              {report.suggestions.map((s, i) => (
                <span key={s}>
                  {i > 0 && ", "}
                  <button className="font-mono font-bold underline" onClick={() => onPickModel(s)}>
                    {s}
                  </button>
                </span>
              ))}
              ?
            </>
          )}
        </div>
      )}
      {report.model_found === true && <div className="text-ok">Model found.</div>}
      {report.models.length > 0 && (
        <details>
          <summary className="cursor-pointer text-muted">{report.models.length} models on the server</summary>
          <ul className="mt-1 max-h-32 overflow-auto font-mono text-[length:var(--fs-xs)] text-paper-dim">
            {report.models.map((m) => (
              <li key={m}>
                <button className="hover:text-brass" onClick={() => onPickModel(m)}>
                  {m}
                </button>
              </li>
            ))}
          </ul>
        </details>
      )}
      <Caps report={report} />
    </div>
  );
}

function ProviderForm({ initial, presets, onDone, onCancel }: { initial: Form; presets: Preset[]; onDone: () => void; onCancel: () => void }) {
  const [f, setF] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<TestReport | null>(null);
  const [busy, setBusy] = useState(false);
  const set = <K extends keyof Form>(k: K, v: Form[K]) => setF((x) => ({ ...x, [k]: v }));

  const persist = async (): Promise<string> => {
    let ref = f.apiKeyRef;
    if (f.keyText.trim()) {
      ref = `secret://provider/${f.id ?? slug(f.name)}/api_key`;
      await routes.putSecret(ref, f.keyText.trim());
    }
    const cfg = toConfig(f, ref);
    const view = f.id ? await routes.updateProvider(f.id, cfg) : await routes.addProvider(cfg);
    setF((x) => ({ ...x, id: view.id, keyText: "", hasKey: view.has_key, apiKeyRef: view.api_key }));
    return view.id;
  };

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  const canSave = f.name.trim() && f.base_url.trim();

  return (
    <div className="space-y-2 rounded-xl border border-brass/40 bg-ink-850 p-3">
      <Field label="Preset">
        <select
          className={field}
          value=""
          onChange={(e) => {
            const p = presets.find((x) => x.name === e.target.value);
            if (p) setF((x) => ({ ...x, base_url: p.base_url, name: x.name || p.name }));
          }}
        >
          <option value="">Choose a preset...</option>
          {presets.map((p) => (
            <option key={p.name} value={p.name}>
              {p.name}
              {p.needs_key ? " (needs a key)" : ""}
            </option>
          ))}
        </select>
      </Field>
      <div className="grid grid-cols-2 gap-2">
        <Field label="Name">
          <input className={field} value={f.name} onChange={(e) => set("name", e.target.value)} />
        </Field>
        <Field label="Model" hint="Blank uses the server's loaded model.">
          <input className={field} value={f.model} onChange={(e) => set("model", e.target.value)} />
        </Field>
      </div>
      <Field label="Base URL">
        <input className={field} value={f.base_url} placeholder="http://127.0.0.1:1234/v1" onChange={(e) => set("base_url", e.target.value)} />
      </Field>
      <Field label="API key" hint="Stored in the vault; never shown again.">
        <input className={field} type="password" autoComplete="new-password" placeholder={f.hasKey ? "Stored (leave blank to keep)" : "Optional"} value={f.keyText} onChange={(e) => set("keyText", e.target.value)} />
      </Field>
      <div className="grid grid-cols-3 gap-2">
        <Field label="Context window">
          <input className={field} type="number" min={1} value={f.context_window} onChange={(e) => set("context_window", e.target.value)} />
        </Field>
        <Field label="Timeout (s)" hint={`Blank: ${DEFAULT_AI_TIMEOUT_SECONDS}`}>
          <input className={field} type="number" min={1} value={f.timeout_s} onChange={(e) => set("timeout_s", e.target.value)} />
        </Field>
        <Field label="Max tool steps" hint={`Blank: ${DEFAULT_AGENT_MAX_STEPS}`}>
          <input className={field} type="number" min={1} value={f.max_tool_steps} onChange={(e) => set("max_tool_steps", e.target.value)} />
        </Field>
      </div>
      <Toggle checked={f.small} onChange={(v) => set("small", v)} label="Small model" hint="Uses a reduced toolset and shorter prompt." />
      <Toggle checked={f.redact_paths} onChange={(v) => set("redact_paths", v)} label="Redact paths" hint="Your user name never leaves the machine in prompts." />
      <details className="rounded-lg border border-line-soft px-2.5 py-1.5">
        <summary className="cursor-pointer font-semibold text-muted">Advanced: params, headers, extra body</summary>
        <div className="mt-2 space-y-2">
          {(
            [
              ["params", "Parameters (JSON)", '{ "temperature": 0.2 }'],
              ["headers", "Headers (JSON)", '{ "X-Org": "abc" }'],
              ["extra_body", "Extra body (JSON)", '{ "reasoning_effort": "low" }'],
            ] as const
          ).map(([k, label, ph]) => (
            <Field key={k} label={label}>
              <textarea className={`${field} min-h-[3.5rem] resize-y font-mono`} spellCheck={false} placeholder={ph} value={f[k]} onChange={(e) => set(k, e.target.value)} />
            </Field>
          ))}
        </div>
      </details>
      <ErrorLine error={error} />
      {report && <Report report={report} onPickModel={(m) => set("model", m)} />}
      <div className="flex flex-wrap justify-end gap-2">
        <button
          className={btnSecondary}
          disabled={!canSave || busy}
          onClick={() =>
            void run(async () => {
              const id = await persist();
              setReport(await routes.testProvider(id));
            })
          }
        >
          {busy ? "Working..." : "Test connection"}
        </button>
        <button className={btnSecondary} onClick={onCancel}>
          Close
        </button>
        <button
          className={btnPrimary}
          disabled={!canSave || busy}
          onClick={() =>
            void run(async () => {
              await persist();
              onDone();
            })
          }
        >
          Save
        </button>
      </div>
    </div>
  );
}

export function ProvidersTab() {
  const [rows, setRows] = useState<ProviderView[] | null>(null);
  const [presets, setPresets] = useState<Preset[]>([]);
  const [form, setForm] = useState<Form | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    routes.providers().then(setRows).catch((e) => setError(errMsg(e)));
  }, []);
  useEffect(() => {
    load();
    routes.presets().then(setPresets).catch(() => {});
  }, [load]);

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <p className="text-muted">The model that answers. Any OpenAI-compatible server works.</p>
        <IconButton title="Add provider" aria-label="Add provider" className="text-brass" onClick={() => setForm(blank())}>
          <PlusIcon />
        </IconButton>
      </div>
      <ErrorLine error={error} />
      {form && !form.id && <ProviderForm initial={form} presets={presets} onCancel={() => { setForm(null); load(); }} onDone={() => { setForm(null); load(); }} />}
      <ul className="space-y-2">
        {rows?.map((p) =>
          form?.id === p.id ? (
            <li key={p.id}>
              <ProviderForm initial={form} presets={presets} onCancel={() => { setForm(null); load(); }} onDone={() => { setForm(null); load(); }} />
            </li>
          ) : (
            <li key={p.id} className={`flex items-center gap-2 rounded-xl border p-2.5 ${p.is_active ? "border-brass/50 bg-brass/5" : "border-line-soft bg-ink-850"}`}>
              <input
                type="radio"
                name="active-provider"
                className="h-4 w-4 accent-[var(--color-brass)]"
                checked={p.is_active}
                aria-label={`Use ${p.name}`}
                onChange={() => void routes.activateProvider(p.id).then(load)}
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-bold">{p.name}</span>
                  {p.small && <span className="rounded-full bg-ink-800 px-2 py-px text-[length:var(--fs-xs)] text-muted">small</span>}
                  {p.is_active && <span className="text-[length:var(--fs-xs)] font-bold text-brass">active</span>}
                </div>
                <div className="truncate font-mono text-[length:var(--fs-xs)] text-faint" title={p.base_url}>
                  {p.model ?? "default model"} · {p.base_url}
                </div>
              </div>
              {confirmId === p.id ? (
                <>
                  <button className={btnDanger} onClick={() => void routes.deleteProvider(p.id).then(() => { setConfirmId(null); load(); })}>
                    Delete
                  </button>
                  <button className={btnSecondary} onClick={() => setConfirmId(null)}>
                    Keep
                  </button>
                </>
              ) : (
                <>
                  <IconButton size="sm" title="Edit provider" aria-label="Edit provider" className="text-faint hover:text-paper" onClick={() => setForm(fromView(p))}>
                    <EditIcon />
                  </IconButton>
                  <IconButton size="sm" title="Delete provider" aria-label="Delete provider" className="text-faint hover:text-err" onClick={() => setConfirmId(p.id)}>
                    <TrashIcon />
                  </IconButton>
                </>
              )}
            </li>
          ),
        )}
      </ul>
      {rows?.length === 0 && !form && <div className="text-muted">No provider yet. Add one to start chatting.</div>}
    </div>
  );
}
