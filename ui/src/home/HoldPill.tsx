import { useEffect, useRef, useState } from "react";
import { useStore } from "../store";

const HOLD_MS = 1200;

// The only red on Home appears while this is held (UI-HOME); letting go early does nothing.
export function HoldPill({ label, warning, onConfirm, disabled }: { label: string; warning: string; onConfirm: () => void; disabled?: boolean }) {
  const [progress, setProgress] = useState(0);
  const raf = useRef<number | null>(null);
  const startedAt = useRef(0);
  const fired = useRef(false);
  const setHolding = useStore((s) => s.setHolding);

  const stop = () => {
    if (raf.current != null) cancelAnimationFrame(raf.current);
    raf.current = null;
    setProgress(0);
    setHolding(false);
  };
  const tick = () => {
    const p = Math.min(1, (performance.now() - startedAt.current) / HOLD_MS);
    setProgress(p);
    if (p >= 1) {
      raf.current = null;
      setHolding(false);
      if (!fired.current) {
        fired.current = true;
        onConfirm();
      }
      return;
    }
    raf.current = requestAnimationFrame(tick);
  };
  const start = () => {
    if (disabled || raf.current != null) return;
    fired.current = false;
    startedAt.current = performance.now();
    setHolding(true);
    raf.current = requestAnimationFrame(tick);
  };
  useEffect(
    () => () => {
      if (raf.current != null) cancelAnimationFrame(raf.current);
      useStore.getState().setHolding(false);
    },
    [],
  );
  const holding = progress > 0 && progress < 1;

  return (
    <span className="relative inline-flex">
      <button
        type="button"
        disabled={disabled}
        data-hold-progress={progress.toFixed(2)}
        className="relative min-w-40 select-none overflow-hidden rounded-full bg-brass px-4 py-1.5 text-[length:var(--fs-base)] font-extrabold text-on-accent touch-none disabled:opacity-40"
        onPointerDown={start}
        onPointerUp={stop}
        onPointerLeave={stop}
        onPointerCancel={stop}
        onKeyDown={(e) => {
          if ((e.key === " " || e.key === "Enter") && !e.repeat) {
            e.preventDefault();
            start();
          }
        }}
        onKeyUp={stop}
        onBlur={stop}
        title="Press and hold"
      >
        <span className="absolute inset-y-0 left-0 bg-err" style={{ width: `${progress * 100}%` }} aria-hidden="true" />
        <span className="relative">{holding ? "Keep holding…" : label}</span>
      </button>
      <span aria-live="polite" className="pointer-events-none absolute right-0 top-full mt-1 whitespace-nowrap text-[length:var(--fs-xs)] font-semibold text-err">
        {holding ? warning : ""}
      </span>
    </span>
  );
}
