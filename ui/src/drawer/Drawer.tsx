import { useEffect } from "react";
import { CloseIcon } from "../components/icons";
import { IconButton } from "../components/IconButton";
import { useStore, type DrawerTab } from "../store";
import { DestinationsTab } from "./DestinationsTab";
import { HistoryTab } from "./HistoryTab";
import { MemoryTab } from "./MemoryTab";
import { ProvidersTab } from "./ProvidersTab";
import { SearchTab } from "./SearchTab";
import { SettingsTab } from "./SettingsTab";

const TABS: { id: DrawerTab; label: string }[] = [
  { id: "memory", label: "Memory" },
  { id: "destinations", label: "Destinations" },
  { id: "providers", label: "Providers" },
  { id: "search", label: "Search" },
  { id: "history", label: "History" },
  { id: "settings", label: "Settings" },
];

export function Drawer() {
  const tab = useStore((s) => s.drawer);
  const open = useStore((s) => s.openDrawer);

  useEffect(() => {
    if (!tab) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && open(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab, open]);

  if (!tab) return null;
  return (
    <div className="fixed inset-0 z-40 flex justify-end">
      <div className="absolute inset-0 bg-black/30" onClick={() => open(null)} aria-hidden />
      <aside className="slide-in relative flex h-full w-full max-w-lg flex-col border-l border-line bg-ink-900 shadow-2xl" role="dialog" aria-label="Settings drawer">
        <div className="flex items-center gap-1 border-b border-line-soft px-3 py-2">
          <h2 className="mr-auto font-[family-name:var(--font-display)] text-[17px] font-extrabold">Settings</h2>
          <IconButton title="Close" aria-label="Close settings" className="text-muted hover:text-paper" onClick={() => open(null)}>
            <CloseIcon />
          </IconButton>
        </div>
        <nav className="flex gap-1 overflow-x-auto border-b border-line-soft px-3 py-2" role="tablist">
          {TABS.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={tab === t.id}
              className={`shrink-0 rounded-full px-3 py-1 text-[length:var(--fs-sm)] font-bold ${tab === t.id ? "bg-brass text-ink-950" : "text-muted hover:bg-ink-800 hover:text-paper"}`}
              onClick={() => open(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
          {tab === "memory" && <MemoryTab />}
          {tab === "destinations" && <DestinationsTab />}
          {tab === "providers" && <ProvidersTab />}
          {tab === "search" && <SearchTab />}
          {tab === "history" && <HistoryTab />}
          {tab === "settings" && <SettingsTab />}
        </div>
      </aside>
    </div>
  );
}
