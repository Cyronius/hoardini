import { Actions } from "./header/Actions";
import { Brand } from "./header/Brand";
import { IndexChip } from "./header/IndexChip";
import { DriveBar } from "./header/Volumes";

export function Header() {
  return (
    <header className="border-b border-line-soft bg-ink-950 px-4 py-2">
      <div className="mx-auto flex w-full max-w-3xl flex-wrap items-center gap-x-5 gap-y-1">
        <Brand />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <DriveBar />
          <IndexChip />
        </div>
        <Actions />
      </div>
    </header>
  );
}
