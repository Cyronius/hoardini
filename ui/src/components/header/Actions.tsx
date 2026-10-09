import { useStore } from "../../store";
import { IconButton } from "../IconButton";
import { GearIcon } from "../icons";
import { TipLine } from "./TipLine";

export function Actions() {
  const connected = useStore((s) => s.connected);
  const openDrawer = useStore((s) => s.openDrawer);
  return (
    <div className="flex flex-col items-end gap-0.5 self-start">
      <div className="relative flex items-center gap-0.5">
        {!connected && <span className="mr-1 text-[length:var(--fs-xs)] font-bold text-err">offline</span>}
        <IconButton title="Settings" aria-label="Open settings" className="text-muted hover:text-paper" onClick={() => openDrawer("memory")}>
          <GearIcon />
        </IconButton>
      </div>
      <TipLine />
    </div>
  );
}
