import { useEffect, useState } from "react";
import { humanBytes } from "../../format";
import { useStore } from "../../store";
import { Mascot, type MascotState } from "../Mascot";
import { systemVolume } from "../mascot/girth";

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

export function Brand() {
  const status = useStore((s) => s.status);
  const mood = useMascotState();
  const vanished = status?.vanished_total_bytes ?? 0;
  return (
    <div className="flex items-center gap-2.5">
      <Mascot state={mood} volume={systemVolume(status?.volumes)} className="h-14 w-14 shrink-0" />
      <div className="leading-tight">
        <div className="font-[family-name:var(--font-display)] text-[17px] font-extrabold tracking-tight">Hoardini</div>
        <div className="text-[length:var(--fs-xs)] text-muted" title="Total space freed by approved jobs">
          made to vanish: <span className="font-bold text-brass">{humanBytes(vanished)}</span>
        </div>
      </div>
    </div>
  );
}
