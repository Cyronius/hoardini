import { useEffect, useRef, useState } from "react";
import { routes } from "../api/client";
import type { OnlineChecks, OnlineState } from "../api/types";
import { useStore } from "../store";

const POLL_MS = 30_000;

export function OnlineChecksConsent({ online, onAnswered }: { online?: OnlineState; onAnswered?: () => void }) {
  const lastRefresh = useStore((s) => s.status?.index.last_refresh_ms);
  const openDrawer = useStore((s) => s.openDrawer);
  const drawer = useStore((s) => s.drawer);
  const [consent, setConsent] = useState<OnlineChecks | null>(null);
  const [waiting, setWaiting] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const given = online != null;
  const drawerUsed = useRef(false);
  const answered = useRef(onAnswered);
  answered.current = onAnswered;

  // The Settings tab's switches also answer, so the settings are read again once the drawer was used.
  useEffect(() => {
    if (drawer) drawerUsed.current = true;
    if (given && (drawer || !drawerUsed.current)) return;
    let live = true;
    routes
      .settings()
      .then((s) => {
        if (!live) return;
        setConsent(s.online_checks);
        if (s.online_checks.asked) answered.current?.();
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [given, lastRefresh, drawer]);

  const asked = consent?.asked ?? online?.asked ?? null;
  useEffect(() => {
    if (given || asked !== false) return;
    const load = () =>
      routes
        .copyChecksWaiting()
        .then((w) => setWaiting(w.online_waiting))
        .catch(() => {});
    void load();
    const t = setInterval(load, POLL_MS);
    return () => clearInterval(t);
  }, [given, asked, lastRefresh]);

  const n = online?.waiting ?? waiting;
  if (asked !== false || n === 0) return null;

  const answer = async (allow: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const s = await routes.putSettings({ online_checks: { asked: true, services: { huggingface: allow, winget: allow } } });
      setConsent(s.online_checks);
      onAnswered?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div role="region" aria-label="Online checks" className="fade-up rounded-2xl border border-info/35 bg-info/8 px-4 py-3 text-[length:var(--fs-sm)]">
      <div>
        <div className="font-bold text-paper">Check big files against Hugging Face and winget?</div>
        <p className="mt-0.5 text-paper-dim">
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
