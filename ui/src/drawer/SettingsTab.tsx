import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { Settings } from "../api/types";
import { humanBytes } from "../format";
import { useStore } from "../store";
import { SYSTEM, THEMES, themeDef, type ThemeDef } from "../theme";
import { btnPrimary, ErrorLine, errMsg, field, Field, Section, Toggle } from "./ui";

const UNITS = [
  { label: "KB", mult: 1024 },
  { label: "MB", mult: 1024 ** 2 },
  { label: "GB", mult: 1024 ** 3 },
] as const;

function splitBytes(n: number): { value: string; unit: number } {
  for (let i = UNITS.length - 1; i >= 0; i--) {
    const u = UNITS[i]!;
    if (n >= u.mult && n % u.mult === 0) return { value: String(n / u.mult), unit: i };
  }
  return { value: String(+(n / UNITS[1].mult).toFixed(2)), unit: 1 };
}

function Swatch({ t, active, onPick }: { t: ThemeDef; active: boolean; onPick: () => void }) {
  const k = t.tokens;
  return (
    <button
      type="button"
      onClick={onPick}
      aria-pressed={active}
      className={`flex items-center gap-2 rounded-xl border px-2 py-1.5 text-left text-[length:var(--fs-sm)] ${active ? "border-brass bg-brass/10" : "border-line-soft hover:border-line"}`}
    >
      <span className="flex h-5 w-8 shrink-0 overflow-hidden rounded-md border border-line">
        <span className="flex-1" style={{ background: k.bg1 }} />
        <span className="flex-1" style={{ background: k.bg3 }} />
        <span className="flex-1" style={{ background: k.accent }} />
      </span>
      <span className="min-w-0 truncate font-semibold">{t.label}</span>
    </button>
  );
}

export function ThemePicker() {
  const pref = useStore((s) => s.themePref);
  const set = useStore((s) => s.setThemePref);
  const light = THEMES.filter((t) => t.mode === "light");
  const dark = THEMES.filter((t) => t.mode === "dark");
  return (
    <div className="space-y-3">
      <button
        type="button"
        aria-pressed={pref === SYSTEM}
        onClick={() => set(SYSTEM)}
        className={`w-full rounded-xl border px-3 py-2 text-left ${pref === SYSTEM ? "border-brass bg-brass/10" : "border-line-soft hover:border-line"}`}
      >
        <div className="font-bold">Follow system</div>
        <div className="text-[length:var(--fs-xs)] text-faint">Marshmallow by day, Marshmallow Night after dark.</div>
      </button>
      {([["Light", light], ["Dark", dark]] as const).map(([title, list]) => (
        <div key={title}>
          <div className="label mb-1 text-[length:var(--fs-xs)]">{title}</div>
          <div className="grid grid-cols-2 gap-1.5">
            {list.map((t) => (
              <Swatch key={t.id} t={t} active={pref === t.id} onPick={() => set(themeDef(t.id).id)} />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

export function SettingsTab() {
  const [s, setS] = useState<Settings | null>(null);
  const [thr, setThr] = useState({ value: "1", unit: 1 });
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    routes
      .settings()
      .then((x) => {
        setS(x);
        setThr(splitBytes(x.threshold_bytes));
      })
      .catch((e) => setError(errMsg(e)));
  }, []);

  if (!s) return <div className="text-muted">{error ? <ErrorLine error={error} /> : <span className="pulse-soft">Loading...</span>}</div>;

  const patch = (p: Partial<Settings>) => {
    setS({ ...s, ...p });
    setSaved(false);
  };
  const save = async () => {
    const bytes = Math.max(0, Math.round(Number(thr.value) * UNITS[thr.unit]!.mult));
    try {
      const { index_size_bytes: _ro, ...rest } = { ...s, threshold_bytes: bytes };
      const out = await routes.putSettings(rest);
      setS(out);
      setThr(splitBytes(out.threshold_bytes));
      setError(null);
      setSaved(true);
    } catch (e) {
      setError(errMsg(e));
    }
  };

  return (
    <div className="space-y-5">
      <Section title="Index">
        <div className="grid grid-cols-[1fr_auto] items-end gap-3">
          <Field label="Index files at or above" hint="Smaller files roll up into their folder's totals. A change applies at the next full refresh.">
            <div className="flex gap-2">
              <input className={field} type="number" min={0} step="any" value={thr.value} onChange={(e) => { setThr({ ...thr, value: e.target.value }); setSaved(false); }} />
              <select className={`${field} !w-20`} value={thr.unit} onChange={(e) => { setThr({ ...thr, unit: Number(e.target.value) }); setSaved(false); }}>
                {UNITS.map((u, i) => (
                  <option key={u.label} value={i}>
                    {u.label}
                  </option>
                ))}
              </select>
            </div>
          </Field>
          <div className="pb-1.5 text-right text-[length:var(--fs-sm)]">
            <div className="text-faint">Index size</div>
            <div className="font-bold tabular-nums" title="Read-only">
              {humanBytes(s.index_size_bytes)}
            </div>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-2">
          <Field label="Indexer workers">
            <input className={field} type="number" min={1} max={32} value={s.indexer_workers} onChange={(e) => patch({ indexer_workers: Math.max(1, Number(e.target.value) || 1) })} />
          </Field>
          <Field label="Indexer memory cap (MB)">
            <input className={field} type="number" min={64} value={s.indexer_memory_cap_mb} onChange={(e) => patch({ indexer_memory_cap_mb: Math.max(64, Number(e.target.value) || 64) })} />
          </Field>
        </div>
      </Section>
      <Section title="Privacy">
        <Toggle checked={s.redact_paths} onChange={(v) => patch({ redact_paths: v })} label="Redact paths sent to the model" hint="Your profile folder becomes ~." />
        <Toggle checked={s.web_search_enabled} onChange={(v) => patch({ web_search_enabled: v })} label="Allow web search" hint="Off by default. Queries are redacted first." />
      </Section>
      <ErrorLine error={error} />
      <div className="flex items-center justify-end gap-3">
        {saved && <span className="text-[length:var(--fs-sm)] font-semibold text-ok">Saved</span>}
        <button className={btnPrimary} onClick={() => void save()}>
          Save settings
        </button>
      </div>
      <Section title="Theme">
        <ThemePicker />
      </Section>
    </div>
  );
}
