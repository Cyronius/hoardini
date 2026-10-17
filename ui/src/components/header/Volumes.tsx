import { useCallback, useEffect, useId, useRef, useState, type FocusEvent, type KeyboardEvent, type RefObject } from "react";
import { routes } from "../../api/client";
import type { VolumeChoice, VolumeView } from "../../api/types";
import { humanBytes } from "../../format";
import { useStore } from "../../store";
import { CheckIcon, ChevronDownIcon, SpinnerIcon } from "../icons";
import { selectedVolume } from "../mascot/girth";

// A watched folder (test setups) shows its last segment; a drive shows "D:".
export const letter = (root: string) => {
  const r = root.replace(/\\$/, "");
  return r.length <= 2 ? r : `…\\${r.split("\\").pop()}`;
};
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

function FreeMeter({ v, className }: { v: Pick<VolumeView, "root" | "total_bytes" | "free_bytes">; className: string }) {
  const pct = v.total_bytes > 0 ? v.free_bytes / v.total_bytes : 0;
  const tone = pct < 0.1 ? "bg-err" : pct < 0.2 ? "bg-brass" : "bg-ok";
  return (
    <div
      className={`h-2 shrink-0 overflow-hidden rounded-full bg-ink-800 ${className}`}
      role="meter"
      aria-valuemin={0}
      aria-valuemax={v.total_bytes}
      aria-valuenow={v.free_bytes}
      aria-label={`${letter(v.root)} free space`}
    >
      <div className={`h-full rounded-full ${tone}`} style={{ width: `${Math.max(2, pct * 100)}%` }} />
    </div>
  );
}

export function DriveBar() {
  const v = useStore((s) => selectedVolume(s.status?.volumes));
  const [open, setOpen] = useState(false);
  const btn = useRef<HTMLButtonElement>(null);
  const menuId = useId();
  const close = useCallback((refocus: boolean) => {
    setOpen(false);
    if (refocus) btn.current?.focus();
  }, []);
  if (!v) return null;
  return (
    <div className="relative flex items-center gap-2 text-[length:var(--fs-sm)]">
      <button
        ref={btn}
        type="button"
        className="-mx-1 flex items-center gap-0.5 rounded-md px-1 font-bold text-paper-dim hover:bg-ink-800 hover:text-paper"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={`Drive ${letter(v.root)}. Choose a drive`}
        title="Choose which drive Hoardini shows"
        onClick={() => setOpen((o) => !o)}
      >
        {letter(v.root)}
        <ChevronDownIcon />
      </button>
      {v.connected ? (
        <div className="flex flex-col gap-1">
          <span className="tabular-nums text-[length:var(--fs-base)] font-semibold text-muted">
            <span className="font-extrabold text-paper">{humanBytes(v.free_bytes, 0)} free</span> of {humanBytes(v.total_bytes, 0)}
          </span>
          <FreeMeter v={v} className="w-48 max-w-full" />
        </div>
      ) : (
        <span className="text-faint">not connected</span>
      )}
      {open && <DriveMenu id={menuId} anchor={btn} onClose={close} />}
    </div>
  );
}

function DriveMenu({ id, anchor, onClose }: { id: string; anchor: RefObject<HTMLButtonElement | null>; onClose: (refocus: boolean) => void }) {
  const setSummary = useStore((s) => s.setSummary);
  const [list, setList] = useState<VolumeView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const focused = useRef(false);

  useEffect(() => {
    let live = true;
    routes
      .volumes()
      .then((l) => live && setList(l))
      .catch((e: unknown) => live && setError(message(e)));
    return () => {
      live = false;
    };
  }, []);

  const buttons = () => Array.from(box.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
  // The confirm step unmounts its own buttons, so focus goes back to the row's Stop watching.
  const cancel = useCallback(() => {
    const root = confirm;
    setConfirm(null);
    if (root) requestAnimationFrame(() => box.current?.querySelector<HTMLButtonElement>(`[data-stop="${CSS.escape(root)}"]`)?.focus());
  }, [confirm]);

  useEffect(() => {
    const down = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!box.current?.contains(t) && !anchor.current?.contains(t)) onClose(false);
    };
    const key = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      if (confirm) cancel();
      else onClose(true);
    };
    document.addEventListener("mousedown", down);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("mousedown", down);
      document.removeEventListener("keydown", key);
    };
  }, [anchor, onClose, confirm, cancel]);

  useEffect(() => {
    if (!list || focused.current) return;
    focused.current = true;
    (box.current?.querySelector<HTMLButtonElement>("[data-current]") ?? buttons()[0])?.focus();
  }, [list]);

  const onKeyDown = (e: KeyboardEvent) => {
    const all = buttons();
    if (!all.length) return;
    const i = all.indexOf(document.activeElement as HTMLButtonElement);
    const go = (n: number) => {
      e.preventDefault();
      all[(n + all.length) % all.length]?.focus();
    };
    if (e.key === "ArrowDown") go(i + 1);
    else if (e.key === "ArrowUp") go(i < 0 ? all.length - 1 : i - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(all.length - 1);
  };

  const onBlur = (e: FocusEvent) => {
    const to = e.relatedTarget as Node | null;
    if (to && !box.current?.contains(to) && !anchor.current?.contains(to)) onClose(false);
  };

  const watched = list?.filter((v) => v.watched).map((v) => v.root) ?? [];
  const selected = list?.find((v) => v.selected)?.root ?? watched[0] ?? "";

  const apply = async (root: string, next: VolumeChoice, closeAfter: boolean) => {
    setBusy(root);
    setError(null);
    try {
      setList(await routes.putVolumes(next));
      setConfirm(null);
      routes.summary().then(setSummary).catch(() => setSummary(null));
      if (closeAfter) onClose(true);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
    }
  };

  const stop = (root: string) => {
    const rest = watched.filter((r) => r !== root);
    void apply(root, { watched: rest, selected: selected === root ? rest[0]! : selected }, false);
  };

  return (
    <div
      ref={box}
      id={id}
      role="dialog"
      aria-label="Drives"
      className="fade-up absolute left-0 top-full z-30 mt-1.5 w-80 max-w-[calc(100vw-2rem)] rounded-2xl border border-line bg-ink-850 p-1.5 shadow-lg"
      onKeyDown={onKeyDown}
      onBlur={onBlur}
    >
      <div className="label px-2 py-1 text-[length:var(--fs-xs)]">Drives</div>
      {list == null && !error && <div className="pulse-soft px-2 py-2 text-muted">Loading…</div>}
      <ul className="max-h-96 overflow-y-auto">
        {list?.map((v) => {
          const name = letter(v.root);
          const last = v.watched && watched.length <= 1;
          const label = (
            <>
              <span className="w-4 shrink-0 text-ok">{v.watched && <CheckIcon />}</span>
              <span className="shrink-0 font-bold text-paper" title={v.root}>
                {name}
              </span>
              <span className="truncate text-muted">{v.label || "Local Disk"}</span>
            </>
          );
          return (
            <li key={v.root} className={`rounded-xl px-2 py-1.5 ${v.selected ? "bg-ink-800" : ""} ${v.connected ? "" : "opacity-60"}`}>
              <div className="flex items-center gap-2">
                {v.watched ? (
                  <button
                    type="button"
                    data-current={v.selected || undefined}
                    aria-current={v.selected || undefined}
                    aria-label={`${name} ${v.label || "Local Disk"}, watched${v.selected ? ", shown" : ""}`}
                    disabled={busy !== null}
                    className="flex min-w-0 flex-1 items-center gap-2 rounded-lg text-left hover:text-paper"
                    onClick={() => (v.selected ? onClose(true) : void apply(v.root, { watched, selected: v.root }, true))}
                  >
                    {label}
                  </button>
                ) : (
                  <div className="flex min-w-0 flex-1 items-center gap-2">{label}</div>
                )}
                {busy === v.root ? (
                  <span className="px-1 text-muted">
                    <SpinnerIcon />
                  </span>
                ) : v.watched ? (
                  <button
                    type="button"
                    className="shrink-0 rounded-full px-2 py-0.5 text-[length:var(--fs-xs)] font-semibold text-muted hover:bg-ink-900 hover:text-err disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-muted"
                    data-stop={v.root}
                    disabled={last || busy !== null}
                    title={last ? "At least one drive stays watched" : `Stop watching ${name} and delete its index`}
                    onClick={() => setConfirm(v.root)}
                  >
                    Stop watching
                  </button>
                ) : (
                  <button
                    type="button"
                    className="shrink-0 rounded-full border border-brass/40 bg-brass/10 px-2.5 py-0.5 text-[length:var(--fs-xs)] font-bold text-brass hover:bg-brass/20 disabled:opacity-40"
                    disabled={busy !== null || !v.connected}
                    title={`Watch ${name}: Hoardini scans it now`}
                    aria-label={`Add ${name}`}
                    onClick={() => void apply(v.root, { watched: [...watched, v.root], selected: v.root }, true)}
                  >
                    Add
                  </button>
                )}
              </div>
              <div className="ml-6 mt-1 flex items-center gap-2 text-[length:var(--fs-xs)]">
                {v.connected ? (
                  <>
                    <FreeMeter v={v} className="w-20" />
                    <span className="tabular-nums text-muted">
                      {humanBytes(v.free_bytes, 0)} free of {humanBytes(v.total_bytes, 0)}
                    </span>
                  </>
                ) : (
                  <span className="text-faint">not connected</span>
                )}
              </div>
              {!v.has_journal && v.fs && (
                <div className="ml-6 mt-0.5 text-[length:var(--fs-xs)] text-faint" title={v.fs}>
                  no change journal: every refresh is a full scan
                </div>
              )}
              {confirm === v.root && (
                <div className="ml-6 mt-1.5 rounded-lg border border-line bg-ink-900 p-2" role="group" aria-label={`Stop watching ${name}`}>
                  <p className="text-[length:var(--fs-sm)] text-paper-dim">Stop watching {name}? Its index and history are deleted. Your files stay.</p>
                  <div className="mt-1.5 flex gap-2 text-[length:var(--fs-xs)]">
                    <button
                      type="button"
                      className="rounded-full border border-err/50 px-2.5 py-0.5 font-bold text-err hover:bg-err/10 disabled:opacity-40"
                      disabled={busy !== null}
                      onClick={() => stop(v.root)}
                    >
                      Stop watching
                    </button>
                    <button type="button" autoFocus className="rounded-full border border-line px-2.5 py-0.5 font-semibold text-muted hover:text-paper" onClick={cancel}>
                      Cancel
                    </button>
                  </div>
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {error && (
        <p role="alert" className="px-2 py-1 text-[length:var(--fs-sm)] text-err">
          {error}
        </p>
      )}
    </div>
  );
}
