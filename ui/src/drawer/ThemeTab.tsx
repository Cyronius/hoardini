import { useStore } from "../store";
import { SYSTEM, THEMES, themeDef, type ThemeDef } from "../theme";

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
