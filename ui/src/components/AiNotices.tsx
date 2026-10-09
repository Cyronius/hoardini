import { useEffect, useState } from "react";
import { useAiSetup } from "../aiSetup";
import { useStore } from "../store";

export function AiNotices() {
  const waitUntil = useAiSetup((s) => s.waitUntil);
  const done = useAiSetup((s) => s.done);
  const open = useAiSetup((s) => s.open);
  const setOpen = useAiSetup((s) => s.setOpen);
  const status = useStore((s) => s.status);
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    if (waitUntil <= Date.now()) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(t);
  }, [waitUntil]);

  const left = Math.ceil((waitUntil - now) / 1000);
  if (left > 0) {
    return (
      <div role="status" className="border-b border-line-soft bg-ink-850 px-4 py-1.5 text-center text-[length:var(--fs-sm)] text-muted">
        Waiting on the free tier... {left}s
      </div>
    );
  }
  if (done && !open && status && !status.active_provider) {
    return (
      <div className="border-b border-line-soft bg-ink-850 px-4 py-1.5 text-center text-[length:var(--fs-sm)] text-muted">
        No AI is set up yet.{" "}
        <button className="font-bold text-brass underline underline-offset-2 hover:no-underline" onClick={() => setOpen(true)}>
          Set up AI
        </button>
      </div>
    );
  }
  return null;
}
