import { useStore } from "../store";
import { Actions } from "./header/Actions";
import { Brand } from "./header/Brand";
import { IndexChip } from "./header/IndexChip";
import { VolumeBar } from "./header/Volumes";

export function Header() {
  const volumes = useStore((s) => s.status?.volumes);
  return (
    <header className="border-b border-line-soft bg-ink-950 px-4 py-2">
      <div className="mx-auto flex w-full max-w-3xl flex-wrap items-center gap-x-5 gap-y-1">
        <Brand />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          {volumes?.map((v) => <VolumeBar key={v.root} root={v.root} total={v.total_bytes} free={v.free_bytes} />)}
          <IndexChip />
        </div>
        <Actions />
      </div>
    </header>
  );
}
