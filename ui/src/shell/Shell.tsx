import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { ChatSession } from "../components/Chat";
import { IconButton } from "../components/IconButton";
import { ChatIcon, PanelCloseIcon } from "../components/icons";
import { selectedVolume } from "../components/mascot/girth";
import { ExploreLinkContext } from "../explore/context";
import { ExploreView } from "../explore/ExploreView";
import { humanBytes } from "../format";
import { ReviewView, useDriveReady } from "../review/ReviewView";
import { useReview } from "../review/state";
import { useStore, type HomeTab } from "../store";
import { CHAT_DEFAULT_PX, CHAT_MIN_PX, chatWidthFor, isWide } from "./layout";

const STEP_PX = 24;

function useWindowWidth(): number {
  const [w, setW] = useState(() => window.innerWidth);
  useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
}

const tabCls = (on: boolean) =>
  `relative -mb-px inline-flex items-center gap-1.5 border-b-2 px-3 pb-2 pt-1.5 text-[length:var(--fs-base)] font-bold transition-colors focus-visible:outline focus-visible:outline-1 focus-visible:outline-brass ${
    on ? "border-brass text-paper" : "border-transparent text-muted hover:text-paper"
  }`;

function Tabs({ wide }: { wide: boolean }) {
  const home = useStore((s) => s.home);
  const chatFront = useStore((s) => s.chatFront);
  const busy = useStore((s) => s.chatBusy);
  const setHome = useStore((s) => s.setHome);
  const showChat = useStore((s) => s.showChat);
  const ready = useDriveReady();
  const front = !wide && chatFront ? "chat" : home;
  const tab = (id: HomeTab | "chat", label: string, extra?: ReactNode) => (
    <button
      key={id}
      type="button"
      role="tab"
      id={`tab-${id}`}
      aria-selected={front === id}
      className={tabCls(front === id)}
      onClick={() => (id === "chat" ? showChat() : setHome(id))}
    >
      {label}
      {extra}
    </button>
  );
  return (
    <nav role="tablist" aria-label="Views" className="flex shrink-0 items-end gap-1 border-b border-line-soft bg-ink-900 px-3 pt-1.5">
      {tab(
        "review",
        "Review",
        ready > 0 && <span className="rounded-full bg-brass px-1.5 py-px text-[length:var(--fs-xs)] tabular-nums text-on-accent">{humanBytes(ready, 0)}</span>,
      )}
      {tab("explore", "Explore")}
      {!wide && tab("chat", "Chat", busy && front !== "chat" && <span className="h-2 w-2 rounded-full bg-brass pulse-soft" aria-label="answering" />)}
    </nav>
  );
}

function Resizer({ width, max, onLive, onDone }: { width: number; max: number; onLive: (w: number | null) => void; onDone: (w: number) => void }) {
  const dragging = useRef(false);
  const fit = (w: number) => chatWidthFor(window.innerWidth, w);
  const down = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    dragging.current = true;
    document.body.classList.add("select-none");
  };
  const moveTo = (e: PointerEvent<HTMLDivElement>) => dragging.current && onLive(fit(window.innerWidth - e.clientX));
  const up = (e: PointerEvent<HTMLDivElement>) => {
    if (!dragging.current) return;
    dragging.current = false;
    document.body.classList.remove("select-none");
    onLive(null);
    onDone(fit(window.innerWidth - e.clientX));
  };
  const key = (e: KeyboardEvent<HTMLDivElement>) => {
    const by = e.key === "ArrowLeft" ? STEP_PX : e.key === "ArrowRight" ? -STEP_PX : 0;
    if (by) {
      e.preventDefault();
      onDone(fit(width + by));
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      onDone(fit(e.key === "Home" ? max : CHAT_MIN_PX));
    }
  };
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Chat width"
      aria-valuenow={width}
      aria-valuemin={CHAT_MIN_PX}
      aria-valuemax={max}
      tabIndex={0}
      title="Drag to resize the chat; double-click for the default width"
      className="group relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize touch-none focus-visible:outline-none"
      onPointerDown={down}
      onPointerMove={moveTo}
      onPointerUp={up}
      onPointerCancel={up}
      onKeyDown={key}
      onDoubleClick={() => onDone(fit(CHAT_DEFAULT_PX))}
    >
      <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-line-soft transition-all group-hover:w-[3px] group-hover:bg-brass/50 group-focus-visible:w-[3px] group-focus-visible:bg-brass" />
    </div>
  );
}

function PanelHead() {
  const provider = useStore((s) => s.status?.active_provider);
  const collapse = useStore((s) => s.setChatCollapsed);
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-line-soft bg-ink-900 px-3 py-1.5">
      <span className="text-brass">
        <ChatIcon />
      </span>
      <span className="font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold text-paper">Chat</span>
      {provider && (
        <span className="min-w-0 truncate text-[length:var(--fs-xs)] text-faint" title={`${provider.name}${provider.model ? ` · ${provider.model}` : ""}`}>
          {provider.model ?? provider.name}
        </span>
      )}
      <IconButton size="sm" className="ml-auto text-muted hover:text-paper" title="Hide the chat" aria-label="Hide the chat" onClick={() => collapse(true)}>
        <PanelCloseIcon />
      </IconButton>
    </div>
  );
}

function Rail() {
  const busy = useStore((s) => s.chatBusy);
  const collapse = useStore((s) => s.setChatCollapsed);
  return (
    <div className="flex w-11 shrink-0 flex-col items-center border-l border-line-soft bg-ink-900 pt-2">
      <button
        type="button"
        className="flex flex-col items-center gap-2 rounded-lg px-1.5 py-2 text-muted hover:bg-ink-800 hover:text-paper focus-visible:outline focus-visible:outline-1 focus-visible:outline-brass"
        title="Show the chat"
        aria-label="Show the chat"
        onClick={() => collapse(false)}
      >
        <ChatIcon />
        <span className="text-[length:var(--fs-xs)] font-bold tracking-wide [writing-mode:vertical-rl]">Chat</span>
        {busy && <span className="h-2 w-2 rounded-full bg-brass pulse-soft" aria-label="answering" />}
      </button>
    </div>
  );
}

export function Shell() {
  const width = useWindowWidth();
  const wide = isWide(width);
  const home = useStore((s) => s.home);
  const chatFront = useStore((s) => s.chatFront);
  const collapsed = useStore((s) => s.chatCollapsed);
  const want = useStore((s) => s.chatWidth);
  const setChatWidth = useStore((s) => s.setChatWidth);
  const nonce = useStore((s) => s.threadNonce);
  const seed = useStore((s) => s.seedMessages);
  const root = useStore((s) => selectedVolume(s.status?.volumes)?.root);
  const exploreAt = useStore((s) => s.exploreAt);
  const openExplore = useStore((s) => s.openExplore);
  const askChat = useStore((s) => s.askChat);
  const [live, setLive] = useState<number | null>(null);

  const link = useMemo(() => ({ onOpenExplore: openExplore }), [openExplore]);
  const onCleanUp = useCallback((path: string) => void useReview.getState().cleanUp(path), []);
  const onWhatIsThis = useCallback((path: string) => useReview.getState().showWhatIs(path), []);
  const onAsk = useCallback((text: string) => askChat(text), [askChat]);

  const panel = live ?? chatWidthFor(width, want);
  const max = chatWidthFor(width, Number.MAX_SAFE_INTEGER);
  const homeShown = wide || !chatFront;
  const chatShown = wide ? !collapsed : chatFront;

  return (
    <ExploreLinkContext.Provider value={link}>
      <div className={`flex min-h-0 flex-1 ${wide ? "flex-row" : "flex-col"}`}>
        <div className={`flex min-w-0 flex-col ${homeShown ? "min-h-0 flex-1" : "flex-none"}`}>
          <Tabs wide={wide} />
          <div className={`min-h-0 flex-1 flex-col ${homeShown ? "flex" : "hidden"}`} role="tabpanel" aria-labelledby={`tab-${home}`}>
            <ReviewView hidden={home !== "review"} />
            {home === "explore" && root && (
              <div className="flex min-h-0 flex-1 flex-col">
                <ExploreView key={exploreAt?.n ?? 0} root={root} focus={exploreAt?.path} onAsk={onAsk} onCleanUp={onCleanUp} onWhatIsThis={onWhatIsThis} />
              </div>
            )}
          </div>
        </div>
        {wide && !collapsed && <Resizer width={panel} max={max} onLive={setLive} onDone={setChatWidth} />}
        <aside
          aria-label="Chat"
          className={`min-h-0 min-w-0 flex-col bg-ink-900 ${chatShown ? "flex" : "hidden"} ${wide ? "shrink-0 border-l border-line-soft" : "flex-1"}`}
          style={wide ? { width: panel } : undefined}
        >
          {wide && <PanelHead />}
          {nonce > 0 ? <ChatSession key={nonce} seed={seed} /> : <div className="flex-1" />}
        </aside>
        {wide && collapsed && <Rail />}
      </div>
    </ExploreLinkContext.Provider>
  );
}
