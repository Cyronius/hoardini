import { useEffect } from "react";
import { routes, streamStatus } from "./api/client";
import { ChatSession } from "./components/Chat";
import { Header } from "./components/Header";
import { Drawer } from "./drawer/Drawer";
import { useStore } from "./store";
import { applyTheme, resolveTheme } from "./theme";

function useThemeSync() {
  const pref = useStore((s) => s.themePref);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => applyTheme(resolveTheme(pref, mq.matches));
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [pref]);
}

function useLiveStatus() {
  const apply = useStore((s) => s.applyStatusEvent);
  const setConnected = useStore((s) => s.setConnected);
  const setSummary = useStore((s) => s.setSummary);
  // Journal refreshes keep the generation but move last_refresh_ms; either one makes the greeting stale.
  const generation = useStore((s) => s.status?.index.generation);
  const lastRefresh = useStore((s) => s.status?.index.last_refresh_ms);

  useEffect(() => {
    routes
      .status()
      .then((st) => apply({ type: "status", ...st }))
      .catch(() => {});
    return streamStatus(apply, setConnected);
  }, [apply, setConnected]);

  useEffect(() => {
    routes.summary().then(setSummary).catch(() => setSummary(null));
  }, [generation, lastRefresh, setSummary]);
}

export function App() {
  useThemeSync();
  useLiveStatus();
  const threadId = useStore((s) => s.threadId);
  const nonce = useStore((s) => s.threadNonce);
  const seed = useStore((s) => s.seedMessages);
  return (
    <div className="flex h-full flex-col bg-ink-900 text-paper">
      <Header />
      <ChatSession key={nonce} threadId={threadId} seed={seed} />
      <Drawer />
    </div>
  );
}
