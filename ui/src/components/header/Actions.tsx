import { useEffect, useState } from "react";
import { routes } from "../../api/client";
import type { ThreadSummary } from "../../api/types";
import { humanAge } from "../../format";
import { useStore } from "../../store";
import { IconButton } from "../IconButton";
import { GearIcon, HistoryIcon, NewChatIcon, TrashIcon } from "../icons";

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

export function Actions() {
  const connected = useStore((s) => s.connected);
  const openDrawer = useStore((s) => s.openDrawer);
  const newThread = useStore((s) => s.newThread);
  const busy = useStore((s) => s.chatBusy);
  const [menu, setMenu] = useState(false);
  return (
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
  );
}
