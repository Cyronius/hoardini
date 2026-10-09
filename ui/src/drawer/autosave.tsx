import { useCallback, useEffect, useRef, useState } from "react";
import { errMsg } from "./ui";

type Status = "idle" | "saving" | "saved" | "error";

// Toggles and switches call `commit`; text fields call `edit` on change and `flush` on blur (UI-SETTINGS-AUTOSAVE).
export function useAutoSave<T>(save: (v: T) => Promise<unknown>) {
  const [status, setStatus] = useState<Status>("idle");
  const [error, setError] = useState<string | null>(null);
  const saveRef = useRef(save);
  saveRef.current = save;
  const pending = useRef<{ v: T } | null>(null);
  const debounce = useRef<number | undefined>(undefined);
  const fade = useRef<number | undefined>(undefined);
  const seq = useRef(0);

  const run = useCallback(async (v: T) => {
    const n = ++seq.current;
    window.clearTimeout(fade.current);
    setStatus("saving");
    setError(null);
    try {
      await saveRef.current(v);
      if (n !== seq.current) return;
      setStatus("saved");
      fade.current = window.setTimeout(() => setStatus("idle"), 2000);
    } catch (e) {
      if (n !== seq.current) return;
      setStatus("error");
      setError(errMsg(e));
    }
  }, []);

  const flush = useCallback(() => {
    window.clearTimeout(debounce.current);
    const p = pending.current;
    pending.current = null;
    if (p) void run(p.v);
  }, [run]);

  const commit = useCallback(
    (v: T) => {
      window.clearTimeout(debounce.current);
      pending.current = null;
      void run(v);
    },
    [run],
  );

  const edit = useCallback(
    (v: T) => {
      pending.current = { v };
      window.clearTimeout(debounce.current);
      debounce.current = window.setTimeout(flush, 500);
    },
    [flush],
  );

  // Closing the drawer mid-edit still saves what was typed.
  useEffect(
    () => () => {
      window.clearTimeout(fade.current);
      flush();
    },
    [flush],
  );

  return { status, error, commit, edit, flush };
}

export function SavedTick({ status }: { status: Status }) {
  if (status === "saving") return <span className="pulse-soft text-[length:var(--fs-xs)] text-faint">Saving…</span>;
  if (status !== "saved") return null;
  return (
    <span className="inline-flex items-center gap-1 text-[length:var(--fs-xs)] font-semibold text-ok" role="status">
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
        <path d="M5 12l5 5L20 7" />
      </svg>
      Saved
    </span>
  );
}
