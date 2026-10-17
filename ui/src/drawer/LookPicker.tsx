import { useState } from "react";
import { useStore } from "../store";
import { DEFAULT_DARK, DEFAULT_LIGHT, SYSTEM, THEMES, groupOf, themeDef, type ThemeDef, type ThemeGroup } from "../theme";

type View = "system" | ThemeGroup;

const VIEWS: { id: View; label: string }[] = [
  { id: "system", label: "Match Windows" },
  { id: "light", label: "Light" },
  { id: "midtone", label: "Midtone" },
  { id: "dark", label: "Dark" },
];

const FIRST: Record<ThemeGroup, string> = {
  light: DEFAULT_LIGHT,
  midtone: THEMES.find((t) => groupOf(t) === "midtone")!.id,
  dark: DEFAULT_DARK,
};

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

export function LookPicker() {
  const pref = useStore((s) => s.themePref);
  const set = useStore((s) => s.setThemePref);
  const [view, setView] = useState<View>(() => (pref === SYSTEM ? "system" : groupOf(themeDef(pref))));
  const choose = (v: View) => {
    setView(v);
    if (v === "system") set(SYSTEM);
    // A group changes the look at once; its swatches then pick within it.
    else if (pref === SYSTEM || groupOf(themeDef(pref)) !== v) set(FIRST[v]);
  };
  return (
    <div className="grid gap-2">
      <div role="tablist" aria-label="Look" className="flex rounded-full bg-ink-800 p-0.5 text-[length:var(--fs-xs)] font-bold">
        {VIEWS.map((v) => (
          <button
            key={v.id}
            type="button"
            role="tab"
            aria-selected={view === v.id}
            className={`flex-auto whitespace-nowrap rounded-full px-2.5 py-1 ${view === v.id ? "bg-ink-900 text-paper shadow-sm" : "text-muted hover:text-paper"}`}
            onClick={() => choose(v.id)}
          >
            {v.label}
          </button>
        ))}
      </div>
      {view === "system" ? (
        <p className="text-[length:var(--fs-xs)] text-muted">Marshmallow while Windows is light, Marshmallow Night while it's dark.</p>
      ) : (
        <div className="grid grid-cols-2 gap-1.5">
          {THEMES.filter((t) => groupOf(t) === view).map((t) => (
            <Swatch key={t.id} t={t} active={pref === t.id} onPick={() => set(t.id)} />
          ))}
        </div>
      )}
    </div>
  );
}
