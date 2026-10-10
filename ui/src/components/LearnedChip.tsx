import { useState } from "react";

const host = (url: string) => {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
};

const button = "rounded-full px-2 py-0.5 font-bold text-brass hover:bg-ink-800";

export function LearnedChip({
  text,
  url,
  onUndo,
  onPin,
  onConfirm,
  confirmed,
  pinned: pinnedAtLoad,
}: {
  text: string;
  url?: string;
  onUndo: () => Promise<unknown>;
  onPin?: (pinned: boolean) => Promise<unknown>;
  onConfirm?: () => Promise<unknown>;
  confirmed?: boolean;
  pinned?: boolean;
}) {
  const [state, setState] = useState<"learned" | "confirmed" | "undone">(confirmed ? "confirmed" : "learned");
  const [pinned, setPinned] = useState(!!pinnedAtLoad);
  const [err, setErr] = useState<string | null>(null);
  const act = async (f: () => Promise<unknown>, after: () => void) => {
    try {
      await f();
      after();
      setErr(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };
  const undone = state === "undone";
  const label = undone ? "undone" : state === "confirmed" ? "learned · confirmed" : pinned ? "learned · pinned" : "learned";
  return (
    <div
      data-learned=""
      className="my-1 flex flex-wrap items-center gap-x-2 rounded-2xl border border-line-soft bg-ink-850 py-0.5 pl-2.5 pr-1 text-[length:var(--fs-sm)]"
    >
      <span className="font-bold text-brass">{label}</span>
      <span className={`min-w-0 flex-1 truncate ${undone ? "text-faint line-through" : "text-muted"}`} title={text}>
        {text}
      </span>
      {url && (
        <a className="shrink-0 text-faint underline-offset-2 hover:text-paper hover:underline" href={url} target="_blank" rel="noreferrer" title={url}>
          {host(url)}
        </a>
      )}
      {!undone && onConfirm && state !== "confirmed" && (
        <button className={button} onClick={() => void act(onConfirm, () => setState("confirmed"))} title="Confirm: save this as something you know, so a later search cannot change it">
          Confirm
        </button>
      )}
      {!undone && onPin && (
        <button
          className={button}
          aria-pressed={pinned}
          onClick={() => void act(() => onPin(!pinned), () => setPinned(!pinned))}
          title={pinned ? "Unpin: let this go when the folder is no longer large" : "Pin: keep this even when the folder is no longer large"}
        >
          {pinned ? "Unpin" : "Pin"}
        </button>
      )}
      {!undone && (
        <button className={button} onClick={() => void act(onUndo, () => setState("undone"))} title="Undo: forget what was learned here" aria-label="Undo learned">
          Undo
        </button>
      )}
      {err && <span className="text-err">{err}</span>}
    </div>
  );
}
