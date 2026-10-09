import type { ReactNode } from "react";

export const field =
  "w-full rounded-lg border border-line bg-ink-900 px-2.5 py-1.5 text-[length:var(--fs-base)] text-paper placeholder:text-faint focus:border-brass focus:outline-none";
export const btnPrimary =
  "rounded-lg bg-brass px-3 py-1.5 text-[length:var(--fs-base)] font-bold text-on-accent hover:opacity-90 disabled:opacity-40";
export const btnSecondary =
  "rounded-lg border border-line px-3 py-1.5 text-[length:var(--fs-base)] font-semibold text-muted hover:text-paper disabled:opacity-40";
export const btnDanger =
  "rounded-lg border border-err/40 px-3 py-1.5 text-[length:var(--fs-base)] font-semibold text-err hover:bg-err/10 disabled:opacity-40";

export function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="block">
      <span className="text-[length:var(--fs-sm)] font-bold text-muted">{label}</span>
      <div className="mt-0.5">{children}</div>
      {hint && <span className="mt-0.5 block text-[length:var(--fs-xs)] text-faint">{hint}</span>}
    </label>
  );
}

export function Toggle({ checked, onChange, label, hint }: { checked: boolean; onChange: (v: boolean) => void; label: string; hint?: string }) {
  return (
    <label className="flex cursor-pointer items-start gap-2.5">
      <input type="checkbox" className="mt-1 h-4 w-4 accent-[var(--color-brass)]" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>
        <span className="font-semibold">{label}</span>
        {hint && <span className="block text-[length:var(--fs-xs)] text-faint">{hint}</span>}
      </span>
    </label>
  );
}

export function Section({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="space-y-2">
      <div className="flex items-center justify-between">
        <h3 className="label">{title}</h3>
        {action}
      </div>
      {children}
    </section>
  );
}

export function ErrorLine({ error }: { error: string | null }) {
  return error ? <div className="rounded-lg border border-err/30 bg-err/10 px-2.5 py-1.5 text-[length:var(--fs-sm)] text-err">{error}</div> : null;
}

export const errMsg = (e: unknown) => (e instanceof Error ? e.message : String(e));
