import { useEffect, useRef, useState, type ReactNode } from "react";
import { ApiError, routes } from "../api/client";
import type { ChildRow, Children } from "../api/types";
import { digits, humanBytes, isUnder, samePath, shortDate } from "../format";
import { ChevronRightIcon, ExplorerIcon } from "./icons";

const TONE = { brass: "bg-brass/70", ok: "bg-ok/70", err: "bg-err/60" } as const;

// Row buttons float over the row's right end on hover or focus, so rows keep one width; an error keeps them up.
export const overlay =
  "absolute right-0 top-1/2 flex -translate-y-1/2 items-center gap-0.5 rounded-md pl-2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 has-[[data-err]]:opacity-100";

export const underOverlay = "group-hover:invisible group-focus-within:invisible group-has-[[data-err]]:invisible";

export const actionBtn =
  "rounded-md px-1.5 py-0.5 text-[length:var(--fs-xs)] font-bold text-brass hover:bg-brass/10 focus-visible:outline focus-visible:outline-1 focus-visible:outline-brass disabled:opacity-40";

export function SizeBar({ fraction, tone = "brass" }: { fraction: number; tone?: keyof typeof TONE }) {
  const pct = Math.max(1, Math.min(100, fraction * 100));
  return (
    <span className="block h-1.5 w-full overflow-hidden rounded-full bg-ink-800" aria-hidden="true">
      <span className={`block h-full rounded-full ${TONE[tone]}`} style={{ width: `${pct}%` }} />
    </span>
  );
}

function revealError(e: unknown): string {
  if (e instanceof ApiError && e.status === 404) return "not found";
  if (e instanceof ApiError && e.status === 403) return "not on a watched drive";
  return e instanceof Error ? e.message : String(e);
}

export function RevealButton({ path, label }: { path: string; label?: string }) {
  const [err, setErr] = useState<string | null>(null);
  const reveal = async () => {
    try {
      await routes.reveal(path);
      setErr(null);
    } catch (e) {
      setErr(revealError(e));
    }
  };
  return (
    <>
      <button type="button" className={`${actionBtn} inline-flex items-center gap-1`} title="Show in Explorer" aria-label={`Show ${path} in Explorer`} onClick={() => void reveal()}>
        <ExplorerIcon />
        {label}
      </button>
      {err && (
        <span data-err className="text-[length:var(--fs-xs)] font-semibold text-err">
          {err}
        </span>
      )}
    </>
  );
}

export function Toggle({ open, onClick, label }: { open: boolean; onClick: () => void; label: string }) {
  return (
    <button
      type="button"
      aria-expanded={open}
      aria-label={`${open ? "Close" : "Open"} ${label}`}
      title={open ? "Hide what is inside" : "Show what is inside"}
      className="grid h-4 w-4 shrink-0 place-items-center rounded text-faint hover:bg-ink-800 hover:text-paper"
      onClick={onClick}
    >
      <span className={`transition-transform duration-150 ${open ? "rotate-90" : ""}`}>
        <ChevronRightIcon />
      </span>
    </button>
  );
}

const pctLabel = (f: number) => (f <= 0 ? "" : f < 0.01 ? "<1%" : `${Math.round(f * 100)}%`);

// The server answers at most this many children per folder (SRV-INDEX-CHILDREN).
const MAX_ROWS = 1000;

export type ItemTreeProps = {
  path: string;
  rootSize?: number;
  actions?: (row: ChildRow) => ReactNode;
  depth?: number;
  selected?: string | null;
  trail?: readonly string[];
  onSelect?: (row: ChildRow) => void;
  onOpen?: (row: ChildRow) => void;
};

type Selecting = Pick<ItemTreeProps, "selected" | "trail" | "onSelect" | "onOpen">;

export function ItemTree({ path, rootSize, actions, depth = 0, selected, trail, onSelect, onOpen }: ItemTreeProps) {
  const [limit, setLimit] = useState(50);
  const [state, setState] = useState<{ data?: Children; err?: string }>({});
  useEffect(() => {
    let live = true;
    routes
      .children(path, limit)
      .then((data) => live && setState({ data }))
      .catch((e) => live && setState({ err: e instanceof Error ? e.message : String(e) }));
    return () => {
      live = false;
    };
  }, [path, limit]);

  const frame = `ml-2 border-l border-line-soft pl-1.5 ${depth === 0 ? "mt-1" : ""}`;
  if (state.err) return <div className={`${frame} py-1 text-[length:var(--fs-xs)] text-err`}>Could not open this folder: {state.err}</div>;
  if (!state.data) return <div className={`${frame} py-1 text-[length:var(--fs-xs)] text-faint pulse-soft`}>Loading...</div>;
  const d = state.data;
  const parent = rootSize ?? d.size;
  return (
    <ul className={`${frame} text-[length:var(--fs-sm)]`}>
      {d.children.map((c) => (
        <TreeRow key={c.path} row={c} parentSize={parent} actions={actions} depth={depth} selected={selected} trail={trail} onSelect={onSelect} onOpen={onOpen} />
      ))}
      {d.small.count > 0 && (
        <li className="flex items-center gap-2 py-[3px] pl-6 text-[length:var(--fs-xs)] italic text-faint">
          {digits(d.small.count)} small file{d.small.count === 1 ? "" : "s"} · {humanBytes(d.small.bytes)}
        </li>
      )}
      {d.more.count > 0 && limit < MAX_ROWS && (
        <li className="pl-5">
          <button type="button" className={actionBtn} onClick={() => setLimit(MAX_ROWS)}>
            +{digits(d.more.count)} more · {humanBytes(d.more.bytes)}
          </button>
        </li>
      )}
      {d.more.count > 0 && limit >= MAX_ROWS && (
        <li className="py-[3px] pl-6 text-[length:var(--fs-xs)] text-faint">
          The biggest {digits(MAX_ROWS)} are listed · {digits(d.more.count)} more · {humanBytes(d.more.bytes)}
        </li>
      )}
      {d.children.length === 0 && d.small.count === 0 && <li className="py-[3px] pl-6 text-[length:var(--fs-xs)] text-faint">Empty</li>}
    </ul>
  );
}

function TreeRow({
  row,
  parentSize,
  actions,
  depth,
  selected,
  trail,
  onSelect,
  onOpen,
}: { row: ChildRow; parentSize: number; actions?: (row: ChildRow) => ReactNode; depth: number } & Selecting) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const frac = parentSize > 0 ? row.size / parentSize : 0;
  const isDir = row.kind === "dir";
  const isSel = selected != null && samePath(row.path, selected);
  const onPath = selected != null && (isUnder(row.path, selected) || !!trail?.some((t) => samePath(t, row.path)));
  useEffect(() => {
    if (onPath) setOpen(true);
  }, [onPath]);
  useEffect(() => {
    if (isSel) ref.current?.scrollIntoView?.({ block: "nearest" });
  }, [isSel]);
  const name = (
    <>
      {row.name}
      {row.denied && <span className="ml-1.5 text-[length:var(--fs-xs)] font-normal text-faint">(some unreadable)</span>}
    </>
  );
  const nameCls = `min-w-0 flex-1 truncate text-left ${isDir ? "font-semibold text-paper" : "text-paper-dim"}`;
  return (
    <li>
      <div
        ref={ref}
        aria-current={isSel || undefined}
        className={`group relative flex items-center gap-2 rounded-md py-[3px] pr-1 ${isSel ? "bg-brass/12 ring-1 ring-inset ring-brass/50" : "hover:bg-ink-800"}`}
      >
        {row.has_children ? <Toggle open={open} onClick={() => setOpen((o) => !o)} label={row.name} /> : <span className="w-4 shrink-0" />}
        {onSelect ? (
          <button type="button" className={`${nameCls} rounded-sm focus-visible:outline focus-visible:outline-1 focus-visible:outline-brass`} title={row.path} onClick={() => onSelect(row)} onDoubleClick={() => onOpen?.(row)}>
            {name}
          </button>
        ) : (
          <span className={nameCls} title={row.path}>
            {name}
          </span>
        )}
        <span className="hidden w-16 shrink-0 @sm:block">
          <SizeBar fraction={frac} />
        </span>
        <span className="w-8 shrink-0 text-right text-[length:var(--fs-xs)] tabular-nums text-faint">{pctLabel(frac)}</span>
        <span className="w-16 shrink-0 text-right tabular-nums">{humanBytes(row.size)}</span>
        <span className={`hidden w-20 shrink-0 text-right text-[length:var(--fs-xs)] tabular-nums text-faint @md:block ${underOverlay}`}>{isDir ? `${digits(row.files)} files` : ""}</span>
        <span className={`hidden w-16 shrink-0 whitespace-nowrap text-right text-[length:var(--fs-xs)] text-faint @lg:block ${underOverlay}`} title="Last modified">
          {shortDate(row.mtime_ms)}
        </span>
        <span className={`${overlay} ${isSel ? "bg-ink-850" : "bg-ink-800"} pr-1`}>
          {actions?.(row)}
          <RevealButton path={row.path} />
        </span>
      </div>
      {open && <ItemTree path={row.path} rootSize={row.size} actions={actions} depth={depth + 1} selected={selected} trail={trail} onSelect={onSelect} onOpen={onOpen} />}
    </li>
  );
}
