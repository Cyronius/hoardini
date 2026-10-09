import { useState } from "react";
import { useStore } from "../store";
import { SYSTEM, THEMES, groupOf, resolveTheme, systemPrefersDark, themeDef, type ThemeDef, type ThemeGroup } from "../theme";

const GROUPS: { id: ThemeGroup; label: string }[] = [
  { id: "dark", label: "Dark" },
  { id: "midtone", label: "Midtone" },
  { id: "light", label: "Light" },
];

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

export function ThemeTab() {
  const pref = useStore((s) => s.themePref);
  const set = useStore((s) => s.setThemePref);
  const [group, setGroup] = useState<ThemeGroup>(() => groupOf(resolveTheme(pref, systemPrefersDark())));
  const list = THEMES.filter((t) => groupOf(t) === group);
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
      <div role="tablist" aria-label="Theme group" className="flex gap-1 rounded-xl border border-line-soft p-1">
        {GROUPS.map((g) => (
          <button
            key={g.id}
            type="button"
            role="tab"
            aria-selected={group === g.id}
            onClick={() => setGroup(g.id)}
            className={`flex-1 rounded-lg px-2 py-1 text-[length:var(--fs-sm)] font-semibold ${group === g.id ? "bg-brass text-on-accent" : "text-muted hover:text-paper"}`}
          >
            {g.label}
          </button>
        ))}
      </div>
      <div className="grid grid-cols-2 gap-1.5">
        {list.map((t) => (
          <Swatch key={t.id} t={t} active={pref === t.id} onPick={() => set(themeDef(t.id).id)} />
        ))}
      </div>
    </div>
  );
}
