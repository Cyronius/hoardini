import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { ThreadSummary } from "../api/types";
import { humanAge, humanBytes } from "../format";
import { useStore } from "../store";
import { GearIcon, HistoryIcon, LeafIcon, NewChatIcon, RefreshIcon, TrashIcon } from "./icons";
import { IconButton } from "./IconButton";
import { Mascot, type MascotState } from "./Mascot";
import { systemVolume } from "./mascot/girth";

function useMascotState(): MascotState {
  const tadaUntil = useStore((s) => s.tadaUntil);
  const chatBusy = useStore((s) => s.chatBusy);
  const runs = useStore((s) => s.status?.running_runs ?? 0);
  const refreshing = useStore((s) => s.status?.index.state === "refreshing");
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const left = tadaUntil - Date.now();
    if (left <= 0) return;
    setNow(Date.now());
    const t = setTimeout(() => setNow(Date.now()), left + 20);
    return () => clearTimeout(t);
  }, [tadaUntil]);
  if (tadaUntil > now) return "tada";
  if (chatBusy || runs > 0) return "think";
  if (refreshing) return "shuffle";
  return "idle";
}

function VolumeBar({ root, total, free }: { root: string; total: number; free: number }) {
  const pct = total > 0 ? free / total : 0;
  const tone = pct < 0.1 ? "bg-err" : pct < 0.2 ? "bg-brass" : "bg-ok";
  return (
    <div className="flex items-center gap-2 text-[length:var(--fs-sm)]" title={`${humanBytes(free)} free of ${humanBytes(total)}`}>
      <span className="font-bold text-paper-dim">{root.replace(/\\$/, "")}</span>
      <div className="h-2 w-24 overflow-hidden rounded-full bg-ink-800" role="meter" aria-valuemin={0} aria-valuemax={total} aria-valuenow={free} aria-label={`${root} free space`}>
        <div className={`h-full rounded-full ${tone}`} style={{ width: `${Math.max(2, pct * 100)}%` }} />
      </div>
      <span className="tabular-nums text-muted">{humanBytes(free, 0)} free</span>
    </div>
  );
}

function IndexChip() {
  const idx = useStore((s) => s.status?.index);
  const [busy, setBusy] = useState(false);
  if (!idx) return null;
  const refreshing = idx.state === "refreshing";
  const pct = idx.progress?.fraction;
  const start = (mode: "background" | "fast") => {
    setBusy(true);
    void routes.refresh(mode).finally(() => setBusy(false));
  };
  const age = idx.last_refresh_ms ? humanAge(Date.now() - idx.last_refresh_ms) : "never";
  return (
    <div className="flex items-center gap-2 text-[length:var(--fs-sm)] text-muted">
      {refreshing ? (
        <>
          {idx.mode === "background" && (
            <span className="text-ok" title="Background mode: low priority, efficiency cores">
              <LeafIcon />
            </span>
          )}
          <span className="text-paper-dim">{idx.mode === "fast" ? "Scanning fast" : "Refreshing"}</span>
          <div className="h-1.5 w-20 overflow-hidden rounded-full bg-ink-800">
            <div
              className={`h-full rounded-full bg-brass transition-all ${pct == null ? "pulse-soft w-1/3" : ""}`}
              style={pct != null ? { width: `${Math.max(3, pct * 100)}%` } : undefined}
            />
          </div>
          <span className="tabular-nums">{pct != null ? `${Math.round(pct * 100)}%` : (idx.progress?.phase ?? "")}</span>
          {idx.mode === "background" && (
            <button
              className="rounded-full border border-brass/40 px-2.5 py-0.5 font-bold text-brass hover:bg-brass/10 disabled:opacity-40"
              disabled={busy}
              onClick={() => start("fast")}
            >
              Scan fast
            </button>
          )}
        </>
      ) : (
        <>
          <span>Index {age}</span>
          <IconButton size="sm" title="Refresh in the background" aria-label="Refresh the index in the background" className="text-muted hover:text-paper" disabled={busy} onClick={() => start("background")}>
            <RefreshIcon />
          </IconButton>
          <button className="rounded-full border border-line px-2.5 py-0.5 font-semibold hover:text-paper disabled:opacity-40" disabled={busy} onClick={() => start("fast")}>
            Scan fast
          </button>
        </>
      )}
    </div>
  );
}

function ThreadsMenu({ onClose }: { onClose: () => void }) {
  const [threads, setThreads] = useState<ThreadSummary[] | null>(null);
  const openThread = useStore((s) => s.openThread);
  const current = useStore((s) => s.threadId);
  useEffect(() => {
    routes.threads().then(setThreads).catch(() => setThreads([]));
  }, []);
  return (
    <div className="absolute right-0 top-10 z-30 w-72 rounded-2xl border border-line bg-ink-850 p-1.5 shadow-lg fade-up">
      <div className="label px-2 py-1 text-[length:var(--fs-xs)]">Recent chats</div>
      {threads == null && <div className="px-2 py-2 text-muted pulse-soft">Loading...</div>}
      {threads?.length === 0 && <div className="px-2 py-2 text-muted">No chats yet.</div>}
      <ul className="max-h-72 overflow-y-auto">
        {threads?.map((t) => (
          <li key={t.id} className="flex items-center">
            <button
              className={`min-w-0 flex-1 rounded-lg px-2 py-1.5 text-left hover:bg-ink-800 ${t.id === current ? "bg-ink-800" : ""}`}
              onClick={() => {
                void openThread(t.id).then(onClose);
              }}
            >
              <div className="truncate font-semibold">{t.title || "Untitled"}</div>
              <div className="text-[length:var(--fs-xs)] text-faint">{humanAge(Date.now() - t.updated_ms)}</div>
            </button>
            <IconButton
              size="sm"
              title="Delete chat"
              aria-label={`Delete chat ${t.title}`}
              className="text-faint hover:text-err"
              onClick={() => {
                void routes.deleteThread(t.id).then(() => setThreads((ts) => ts?.filter((x) => x.id !== t.id) ?? null));
              }}
            >
              <TrashIcon />
            </IconButton>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function Header() {
  const status = useStore((s) => s.status);
  const connected = useStore((s) => s.connected);
  const openDrawer = useStore((s) => s.openDrawer);
  const newThread = useStore((s) => s.newThread);
  const busy = useStore((s) => s.chatBusy);
  const [menu, setMenu] = useState(false);
  const mood = useMascotState();
  const vanished = status?.vanished_total_bytes ?? 0;

  return (
    <header className="border-b border-line-soft bg-ink-950 px-4 py-2">
      <div className="mx-auto flex w-full max-w-3xl flex-wrap items-center gap-x-5 gap-y-1">
        <div className="flex items-center gap-2.5">
          <Mascot state={mood} volume={systemVolume(status?.volumes)} className="h-14 w-14 shrink-0" />
          <div className="leading-tight">
            <div className="font-[family-name:var(--font-display)] text-[17px] font-extrabold tracking-tight">Hoardini</div>
            <div className="text-[length:var(--fs-xs)] text-muted" title="Total space freed by approved jobs">
              made to vanish: <span className="font-bold text-brass">{humanBytes(vanished)}</span>
            </div>
          </div>
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          {status?.volumes.map((v) => <VolumeBar key={v.root} root={v.root} total={v.total_bytes} free={v.free_bytes} />)}
          <IndexChip />
        </div>
        <div className="relative flex items-center gap-0.5 self-start">
          {!connected && <span className="mr-1 text-[length:var(--fs-xs)] font-bold text-err">offline</span>}
          <IconButton title="New chat" aria-label="New chat" className="text-muted hover:text-paper" disabled={busy} onClick={newThread}>
            <NewChatIcon />
          </IconButton>
          <IconButton title="Recent chats" aria-label="Recent chats" className="text-muted hover:text-paper" disabled={busy} onClick={() => setMenu((m) => !m)}>
            <HistoryIcon />
          </IconButton>
          <IconButton title="Settings" aria-label="Open settings" className="text-muted hover:text-paper" onClick={() => openDrawer("memory")}>
            <GearIcon />
          </IconButton>
          {menu && <ThreadsMenu onClose={() => setMenu(false)} />}
        </div>
      </div>
    </header>
  );
}
