import { Actions } from "./header/Actions";
import { Brand } from "./header/Brand";
import { IndexChip } from "./header/IndexChip";
import { DriveBar } from "./header/Volumes";

export function Header() {
  return (
    <header className="border-b border-line-soft bg-ink-950 px-4 py-2">
      <div className="flex w-full flex-wrap items-center gap-x-5 gap-y-1">
        <Brand />
        {/* On a phone-width window the drive bar takes its own line instead of squeezing between brand and settings. */}
        <div className="order-3 flex min-w-0 grow basis-full flex-col gap-1 sm:order-2 sm:basis-0">
          <DriveBar />
          <IndexChip />
        </div>
        <div className="order-2 ml-auto self-start sm:order-3">
          <Actions />
        </div>
      </div>
    </header>
  );
}
