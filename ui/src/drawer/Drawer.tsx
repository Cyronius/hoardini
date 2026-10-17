import { useEffect, useState, type ReactNode } from "react";
import { routes } from "../api/client";
import type { Destination } from "../api/types";
import { useAiSetup } from "../aiSetup";
import { CloseIcon } from "../components/icons";
import { IconButton } from "../components/IconButton";
import { TipLine } from "../components/header/TipLine";
import { fileName, humanBytes } from "../format";
import { useReview } from "../review/state";
import { useStore, type DrawerTab } from "../store";
import { DestinationsTab } from "./DestinationsTab";
import { LookPicker } from "./LookPicker";
import { MemoryTab } from "./MemoryTab";
import { ProvidersTab } from "./ProvidersTab";
import { SearchTab } from "./SearchTab";
import { SettingsTab } from "./SettingsTab";

// Settings is four rows; everything else sits one level down under Advanced (UI-SETTINGS).
const ADVANCED: { id: Exclude<DrawerTab, "main">; label: string; hint: string }[] = [
  { id: "providers", label: "AI providers", hint: "Set one up by hand, or change its model" },
  { id: "destinations", label: "Places to move things", hint: "Other drives, network folders and cloud storage" },
  { id: "search", label: "Web lookups", hint: "Let Hoardini search the web to identify things" },
  { id: "memory", label: "Hoardini's notes", hint: "What Hoardini has learned about your folders" },
  { id: "settings", label: "Scanning", hint: "How fast Hoardini scans, and its history" },
];

const link = "shrink-0 font-bold text-brass hover:underline";

function Row({ k, v, action }: { k: string; v: ReactNode; action?: ReactNode }) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5 border-t border-line-soft py-3">
      <div className="min-w-0">
        <div className="font-extrabold">{k}</div>
        <div className="text-[length:var(--fs-sm)] text-muted">{v}</div>
      </div>
      {action}
    </div>
  );
}

function Main() {
  const open = useStore((s) => s.openDrawer);
  const provider = useStore((s) => s.status?.active_provider);
  const areas = useReview((s) => s.main?.areas);
  const [dests, setDests] = useState<Destination[] | null>(null);
  const [showKept, setShowKept] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  useEffect(() => {
    routes.destinations().then(setDests, () => setDests([]));
  }, []);
  const kept = (areas ?? []).filter((a) => a.user && a.verdict === "keep");
  const bringBack = async (path: string) => {
    setBusy(path);
    try {
      await routes.forgetAnswer(path);
      await useReview.getState().runNow();
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className="grid">
      <Row
        k="Brain"
        v={provider ? `${provider.name}${provider.model ? ` (${provider.model})` : ""}` : "Not set up yet"}
        action={
          <button
            type="button"
            className={link}
            onClick={() => {
              open(null);
              useAiSetup.getState().setOpen(true);
            }}
          >
            Change
          </button>
        }
      />
      <Row
        k="Move big things to"
        v={dests == null ? "…" : dests.length ? dests.map((d) => d.name).join(", ") : "Not set up. Adds a Move button for your own files."}
        action={
          <button type="button" className={link} onClick={() => open("destinations")}>
            {dests?.length ? "Change" : "Set up"}
          </button>
        }
      />
      <Row
        k="Things you kept"
        v={kept.length ? `${kept.length} ${kept.length === 1 ? "thing" : "things"} Hoardini won't suggest again` : "Nothing yet"}
        action={
          kept.length > 0 && (
            <button type="button" className={link} aria-expanded={showKept} onClick={() => setShowKept((s) => !s)}>
              {showKept ? "Hide" : "Show"}
            </button>
          )
        }
      />
      {showKept && (
        <ul className="grid pb-2">
          {kept.map((a) => (
            <li key={a.path} className="flex items-center gap-3 py-1 text-[length:var(--fs-sm)]">
              <span className="min-w-0 flex-1 truncate" title={a.path}>
                <span className="font-semibold">{fileName(a.path) || a.path}</span> <span className="text-muted">· {humanBytes(a.size)}</span>
              </span>
              <button type="button" className={`${link} disabled:opacity-40`} disabled={busy != null} onClick={() => void bringBack(a.path)}>
                {busy === a.path ? "Bringing back…" : "Bring back"}
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="grid gap-2 border-t border-line-soft py-3">
        <div className="font-extrabold">Look</div>
        <LookPicker />
      </div>
      <div className="mt-4 border-t border-line-soft pt-3">
        <div className="mb-1 text-[length:var(--fs-xs)] font-bold uppercase tracking-wide text-faint">Advanced</div>
        <ul className="grid">
          {ADVANCED.map((a) => (
            <li key={a.id}>
              <button type="button" className="grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-lg px-1 py-2 text-left hover:bg-ink-850" onClick={() => open(a.id)}>
                <span>
                  <span className="block font-bold">{a.label}</span>
                  <span className="block text-[length:var(--fs-xs)] text-muted">{a.hint}</span>
                </span>
                <span className="text-faint" aria-hidden="true">
                  ›
                </span>
              </button>
            </li>
          ))}
        </ul>
      </div>
      <div className="mt-4">
        <TipLine />
      </div>
    </div>
  );
}

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
  const page = ADVANCED.find((a) => a.id === tab);
  return (
    <div className="fixed inset-0 z-40 flex justify-end">
      <div className="absolute inset-0 bg-black/30" onClick={() => open(null)} aria-hidden />
      <aside className="slide-in relative flex h-full w-full max-w-md flex-col border-l border-line bg-ink-900 shadow-2xl" role="dialog" aria-label="Settings">
        <div className="flex items-center gap-2 border-b border-line-soft px-4 py-2.5">
          {page && (
            <button type="button" className="font-bold text-brass hover:underline" onClick={() => open("main")}>
              ← Settings
            </button>
          )}
          <h2 className="mr-auto font-[family-name:var(--font-display)] text-[19px] font-bold">{page ? page.label : "Settings"}</h2>
          <IconButton title="Close" aria-label="Close settings" className="text-muted hover:text-paper" onClick={() => open(null)}>
            <CloseIcon />
          </IconButton>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-3">
          {tab === "main" && <Main />}
          {tab === "memory" && <MemoryTab />}
          {tab === "destinations" && <DestinationsTab />}
          {tab === "providers" && <ProvidersTab />}
          {tab === "search" && <SearchTab />}
          {tab === "settings" && <SettingsTab />}
        </div>
      </aside>
    </div>
  );
}
