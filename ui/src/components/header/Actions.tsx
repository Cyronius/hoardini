import { useStore } from "../../store";
import { IconButton } from "../IconButton";
import { FreshStartIcon, GearIcon } from "../icons";
import { TipLine } from "./TipLine";

export function Actions() {
  const connected = useStore((s) => s.connected);
  const openDrawer = useStore((s) => s.openDrawer);
  const startFresh = useStore((s) => s.startFresh);
  const busy = useStore((s) => s.chatBusy);
  return (
    <div className="flex flex-col items-end gap-0.5 self-start">
      <div className="relative flex items-center gap-0.5">
        {!connected && <span className="mr-1 text-[length:var(--fs-xs)] font-bold text-err">offline</span>}
        <IconButton
          title="Start fresh: Hoardini forgets the chat so far (it stays on screen)"
          aria-label="Start fresh"
          className="text-muted hover:text-paper"
          disabled={busy || !connected}
          onClick={() => void startFresh().catch(() => {})}
        >
          <FreshStartIcon />
        </IconButton>
        <IconButton title="Settings" aria-label="Open settings" className="text-muted hover:text-paper" onClick={() => openDrawer("memory")}>
          <GearIcon />
        </IconButton>
      </div>
      <TipLine />
    </div>
  );
}
