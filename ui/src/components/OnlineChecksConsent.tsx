import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { OnlineChecks } from "../api/types";
import { useStore } from "../store";

const POLL_MS = 30_000;

// Asked once, when online candidates wait and the user has not answered yet (EVID-COPY-CONSENT).
export function OnlineChecksConsent({ waiting: given }: { waiting?: number }) {
  const lastRefresh = useStore((s) => s.status?.index.last_refresh_ms);
  const openDrawer = useStore((s) => s.openDrawer);
  const drawer = useStore((s) => s.drawer);
  const [consent, setConsent] = useState<OnlineChecks | null>(null);
  const [waiting, setWaiting] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    routes
      .settings()
      .then((s) => setConsent(s.online_checks))
      .catch(() => {});
  }, [lastRefresh, drawer]);

  useEffect(() => {
    if (given != null || !consent || consent.asked) return;
    const load = () =>
      routes
        .copyChecksWaiting()
        .then((w) => setWaiting(w.online_waiting))
        .catch(() => {});
    void load();
    const t = setInterval(load, POLL_MS);
    return () => clearInterval(t);
  }, [given, consent, lastRefresh]);

  const n = given ?? waiting;
  if (!consent || consent.asked || n === 0) return null;

  const answer = async (allow: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const s = await routes.putSettings({ online_checks: { asked: true, services: { huggingface: allow, winget: allow } } });
      setConsent(s.online_checks);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div role="region" aria-label="Online checks" className="border-b border-line-soft bg-ink-850 px-4 py-2 text-[length:var(--fs-sm)]">
      <div className="mx-auto max-w-3xl">
        <div className="font-bold">Check big files against Hugging Face and winget?</div>
        <p className="mt-0.5 text-muted">
          Hoardini can confirm that {n} big {n === 1 ? "file" : "files"} can be downloaded again by looking them up online. It sends file names and
          package or repo names, never file contents or hashes: the published hash is fetched and compared on this PC. Checks against your own copies,
          git, and cloud sync always run.
        </p>
        <div className="mt-1.5 flex flex-wrap items-center gap-2">
          <button type="button" disabled={busy} className="rounded-lg bg-brass px-3 py-1 font-bold text-on-accent hover:opacity-90 disabled:opacity-40" onClick={() => void answer(true)}>
            Allow
          </button>
          <button type="button" disabled={busy} className="rounded-lg border border-line px-3 py-1 font-semibold text-muted hover:text-paper disabled:opacity-40" onClick={() => void answer(false)}>
            Not now
          </button>
          <button type="button" className="font-semibold text-brass underline underline-offset-2 hover:no-underline" onClick={() => openDrawer("settings")}>
            Choose in Settings
          </button>
          {error && <span className="text-err">{error}</span>}
        </div>
      </div>
    </div>
  );
}
