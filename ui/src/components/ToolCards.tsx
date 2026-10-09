import { useState, type ReactNode } from "react";
import type { CardData, ChangeRow, ChangesData, FilesData, FoldersData } from "../api/types";
import { digits, fileName, humanAge, humanBytes, isUnder, samePath, shortDate, signedBytes, sinceLabel } from "../format";
import { useStore } from "../store";
import { ItemTree, RevealButton, SizeBar, Toggle, actionBtn, overlay, underOverlay } from "./ItemTree";
import { ChangesChart, RANGE_RE } from "./ToolChip";

const SHOWN = 8;

export const askAbout = (path: string) => `What is ${path} and is it safe to remove?`;
export const cleanUpIn = (path: string) => `What can I clean up in ${path}?`;

export function isCardData(d: unknown): d is CardData {
  const k = (d as { kind?: unknown; v?: unknown } | null)?.kind;
  return (d as { v?: unknown } | null)?.v === 1 && (k === "folders" || k === "files" || k === "changes");
}

type Ask = (text: string) => void;

const parentOf = (p: string) => p.replace(/\\[^\\]+$/, "");

// The row already sits under its scope, so only the part below the scope says where it is.
function where(path: string, scope: string | null): string {
  const parent = parentOf(path);
  if (!scope) return /^([a-z]:|~)$/i.test(parent) ? "" : parent;
  if (samePath(parent, scope)) return "";
  return isUnder(scope, parent) ? parent.slice(scope.replace(/\\+$/, "").length + 1) : parent;
}

export function AskButtons({ path, dir, onAsk }: { path: string; dir: boolean; onAsk: Ask }) {
  const busy = useStore((s) => s.chatBusy);
  return (
    <>
      <button type="button" className={actionBtn} disabled={busy} title={askAbout(path)} onClick={() => onAsk(askAbout(path))}>
        Ask
      </button>
      {dir && (
        <button type="button" className={actionBtn} disabled={busy} title={cleanUpIn(path)} onClick={() => onAsk(cleanUpIn(path))}>
          Clean up
        </button>
      )}
    </>
  );
}

function RankRow({
  path,
  size,
  frac,
  meta,
  metaTitle,
  scope,
  dir,
  note,
  onAsk,
}: {
  path: string;
  size: number;
  frac: number;
  meta: string;
  metaTitle?: string;
  scope: string | null;
  dir: boolean;
  note?: string;
  onAsk: Ask;
}) {
  const [open, setOpen] = useState(false);
  const name = fileName(path) || path;
  const at = where(path, scope);
  return (
    <li className="py-1.5">
      <div className="group flex items-center gap-2">
        {dir ? <Toggle open={open} onClick={() => setOpen((o) => !o)} label={name} /> : <span className="w-4 shrink-0" />}
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <span className="min-w-0 truncate font-semibold text-paper" title={path}>
              {name}
            </span>
            {at && (
              <span className="min-w-0 truncate text-[length:var(--fs-xs)] text-faint" title={path}>
                {at}
              </span>
            )}
            <span className="ml-auto shrink-0 font-bold tabular-nums text-paper">{humanBytes(size)}</span>
          </div>
          <div className="relative mt-1 flex items-center gap-2">
            <span className="min-w-0 flex-1">
              <SizeBar fraction={frac} />
            </span>
            <span className={`shrink-0 text-[length:var(--fs-xs)] tabular-nums text-faint ${underOverlay}`} title={metaTitle}>
              {meta}
              {note && <span className="ml-1.5 text-brass">{note}</span>}
            </span>
            <span className={`${overlay} bg-ink-850`}>
              <AskButtons path={path} dir={dir} onAsk={onAsk} />
              <RevealButton path={path} />
            </span>
          </div>
        </div>
      </div>
      {open && <ItemTree path={path} rootSize={size} actions={(r) => <AskButtons path={r.path} dir={r.kind === "dir"} onAsk={onAsk} />} />}
    </li>
  );
}

function Footer({ total, all, onAll, more }: { total: number; all: boolean; onAll: () => void; more: { count: number; bytes: number } }) {
  if (total <= SHOWN && more.count === 0) return null;
  return (
    <div className="mt-1 flex items-center gap-2 border-t border-line-soft pt-1.5 text-[length:var(--fs-xs)] text-faint">
      {total > SHOWN && !all && (
        <button type="button" className={actionBtn} onClick={onAll}>
          Show all {total}
        </button>
      )}
      {more.count > 0 && (
        <span className="ml-auto tabular-nums">
          +{digits(more.count)} more{more.bytes > 0 ? ` · ${humanBytes(more.bytes)}` : ""}
        </span>
      )}
    </div>
  );
}

function FoldersBody({ data, onAsk }: { data: FoldersData; onAsk: Ask }) {
  const [all, setAll] = useState(false);
  const max = Math.max(1, ...data.items.map((i) => i.size));
  const shown = all ? data.items : data.items.slice(0, SHOWN);
  if (data.items.length === 0) return <p className="text-[length:var(--fs-sm)] text-muted">No subfolders.</p>;
  return (
    <>
      <ul className="divide-y divide-line-soft">
        {shown.map((d) => (
          <RankRow
            key={d.path}
            path={d.path}
            size={d.size}
            frac={d.size / max}
            meta={`${digits(d.files)} files`}
            note={d.denied ? "some unreadable" : undefined}
            scope={data.under}
            dir
            onAsk={onAsk}
          />
        ))}
      </ul>
      <Footer total={data.items.length} all={all} onAll={() => setAll(true)} more={data.more} />
    </>
  );
}

function FilesBody({ data, onAsk }: { data: FilesData; onAsk: Ask }) {
  const [all, setAll] = useState(false);
  const max = Math.max(1, ...data.items.map((i) => i.size));
  const shown = all ? data.items : data.items.slice(0, SHOWN);
  if (data.items.length === 0) return <p className="text-[length:var(--fs-sm)] text-muted">No files match.</p>;
  return (
    <>
      <ul className="divide-y divide-line-soft">
        {shown.map((f) => (
          <RankRow
            key={f.path}
            path={f.path}
            size={f.size}
            frac={f.size / max}
            meta={f.modified_ms ? `modified ${shortDate(f.modified_ms)}` : ""}
            metaTitle={f.accessed_ms ? `Last opened ${shortDate(f.accessed_ms)}` : undefined}
            scope={data.filters.under}
            dir={false}
            onAsk={onAsk}
          />
        ))}
      </ul>
      <Footer total={data.items.length} all={all} onAll={() => setAll(true)} more={data.more} />
    </>
  );
}

type Nested = { row: ChangeRow; depth: number };

// A row under another listed row is shown beneath it, so a folder and the file that explains it read as one.
function nest(rows: ChangeRow[]): Nested[] {
  const sorted = [...rows].sort((a, b) => Math.abs(b.delta) - Math.abs(a.delta));
  const parent = (r: ChangeRow) =>
    sorted.filter((o) => o !== r && isUnder(o.path, r.path)).sort((a, b) => b.path.length - a.path.length)[0] ?? null;
  const kids = new Map<ChangeRow | null, ChangeRow[]>();
  for (const r of sorted) {
    const p = parent(r);
    kids.set(p, [...(kids.get(p) ?? []), r]);
  }
  const out: Nested[] = [];
  const walk = (p: ChangeRow | null, depth: number) => {
    for (const c of kids.get(p) ?? []) {
      out.push({ row: c, depth });
      walk(c, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

const topLevel = (rows: ChangeRow[]) => rows.filter((r) => !rows.some((o) => o !== r && isUnder(o.path, r.path)));
const total = (rows: ChangeRow[]) => topLevel(rows).reduce((a, r) => a + Math.abs(r.delta), 0);

function ChangeItem({ n, max, tone, volume, onAsk }: { n: Nested; max: number; tone: "ok" | "err"; volume: string; onAsk: Ask }) {
  const [open, setOpen] = useState(false);
  const { row, depth } = n;
  const gone = row.after == null;
  const dir = row.kind === "dir";
  const name = fileName(row.path) || row.path;
  const at = depth === 0 ? where(row.path, volume) : "";
  const span = row.before != null && row.after != null ? `${humanBytes(row.before)} → ${humanBytes(row.after)}` : gone ? "removed" : "new";
  return (
    <li className="py-1.5" style={{ paddingLeft: depth * 16 }}>
      <div className={`group flex items-center gap-2 ${depth ? "border-l border-line-soft pl-2" : ""}`}>
        {dir && !gone ? <Toggle open={open} onClick={() => setOpen((o) => !o)} label={name} /> : <span className="w-4 shrink-0" />}
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <span className={`min-w-0 truncate font-semibold ${gone ? "text-muted line-through decoration-faint/60" : "text-paper"}`} title={row.path}>
              {name}
            </span>
            {at && (
              <span className="min-w-0 truncate text-[length:var(--fs-xs)] text-faint" title={row.path}>
                {at}
              </span>
            )}
            <span className={`ml-auto shrink-0 font-bold tabular-nums ${tone === "ok" ? "text-ok" : "text-err"}`}>{signedBytes(row.delta)}</span>
          </div>
          <div className="relative mt-1 flex items-center gap-2">
            <span className="min-w-0 flex-1">
              <SizeBar fraction={Math.abs(row.delta) / max} tone={tone} />
            </span>
            <span className={`shrink-0 text-[length:var(--fs-xs)] tabular-nums text-faint ${gone ? "" : underOverlay}`}>{span}</span>
            {!gone && (
              <span className={`${overlay} bg-ink-850`}>
                <AskButtons path={row.path} dir={dir} onAsk={onAsk} />
                <RevealButton path={row.path} />
              </span>
            )}
          </div>
        </div>
      </div>
      {open && <ItemTree path={row.path} rootSize={row.after ?? undefined} actions={(r) => <AskButtons path={r.path} dir={r.kind === "dir"} onAsk={onAsk} />} />}
    </li>
  );
}

function ChangeSection({ title, rows, tone, volume, onAsk }: { title: string; rows: ChangeRow[]; tone: "ok" | "err"; volume: string; onAsk: Ask }) {
  const [all, setAll] = useState(false);
  if (rows.length === 0) return null;
  const nested = nest(rows);
  const max = Math.max(1, ...rows.map((r) => Math.abs(r.delta)));
  const shown = all ? nested : nested.slice(0, SHOWN);
  return (
    <div className="mt-2">
      <div className="label text-[length:var(--fs-xs)] uppercase">{title}</div>
      <ul className="divide-y divide-line-soft">
        {shown.map((n) => (
          <ChangeItem key={n.row.path} n={n} max={max} tone={tone} volume={volume} onAsk={onAsk} />
        ))}
      </ul>
      {nested.length > SHOWN && !all && (
        <button type="button" className={`${actionBtn} mt-1`} onClick={() => setAll(true)}>
          Show all {nested.length}
        </button>
      )}
    </div>
  );
}

function ChangesBody({ data, onAsk }: { data: ChangesData; onAsk: Ask }) {
  const keep = (c: ChangeRow) => !samePath(c.path, data.volume);
  const freed = [...data.removed, ...data.shrank].filter(keep);
  const grew = [...data.grew, ...data.new].filter(keep);
  const omitted = data.omitted.grew + data.omitted.new + data.omitted.shrank + data.omitted.removed;
  return (
    <>
      <ChangesChart range={{ since: data.range_ms[0], until: data.range_ms[1], volume: data.volume }} />
      {freed.length + grew.length > 0 && (
        <div className="mt-1 text-[length:var(--fs-sm)] text-muted">
          Freed <span className="font-bold text-ok tabular-nums">{humanBytes(total(freed))}</span> · Grew{" "}
          <span className="font-bold text-err tabular-nums">{humanBytes(total(grew))}</span>
        </div>
      )}
      <ChangeSection title="Freed" rows={freed} tone="ok" volume={data.volume} onAsk={onAsk} />
      <ChangeSection title="Grew" rows={grew} tone="err" volume={data.volume} onAsk={onAsk} />
      {freed.length + grew.length === 0 && <p className="mt-1 text-[length:var(--fs-sm)] text-muted">No big changes.</p>}
      {omitted > 0 && <div className="mt-1 text-right text-[length:var(--fs-xs)] text-faint">+{digits(omitted)} smaller changes</div>}
    </>
  );
}

function changesHeadline(d: ChangesData): ReactNode {
  const vol = d.volume.replace(/\\+$/, "");
  const since = sinceLabel(d.from.ms);
  const fd = d.free_delta;
  if (fd == null) return `Changes on ${vol} since ${since}`;
  if (fd === 0) return `${vol} has the same free space as at ${since}`;
  return (
    <>
      {vol} has{" "}
      <span className={fd > 0 ? "text-ok" : "text-err"}>
        {humanBytes(Math.abs(fd))} {fd > 0 ? "more" : "less"} free
      </span>{" "}
      since {since}
    </>
  );
}

function filtersLabel(d: FilesData): string {
  const f = d.filters;
  const parts = [f.min_size ? `at least ${humanBytes(f.min_size)}` : "", f.ext ? f.ext : "", f.not_used_for_days ? `unused ${f.not_used_for_days}+ days` : ""];
  return parts.filter(Boolean).join(" · ");
}

export function ToolCard({ name, args, result, data, onAsk }: { name: string; args: Record<string, unknown>; result: string; data: CardData; onAsk: Ask }) {
  const [raw, setRaw] = useState(false);
  let title: ReactNode;
  let scope = "";
  let body: ReactNode;
  if (data.kind === "folders") {
    title = "Largest folders";
    scope = data.under ?? "on your drives";
    body = <FoldersBody data={data} onAsk={onAsk} />;
  } else if (data.kind === "files") {
    title = "Largest files";
    scope = [data.filters.under ?? "on your drives", filtersLabel(data)].filter(Boolean).join(" · ");
    body = <FilesBody data={data} onAsk={onAsk} />;
  } else {
    title = changesHeadline(data);
    body = <ChangesBody data={data} onAsk={onAsk} />;
  }
  const asOf = data.kind === "changes" ? null : data.as_of_ms;
  const text = name === "changes" ? result.replace(RANGE_RE, "").trimEnd() : result;
  return (
    <section className="@container fade-up my-2 rounded-2xl border border-line bg-ink-850 px-3 pb-2 pt-2.5" aria-label={typeof title === "string" ? title : "What changed"}>
      <header className="flex items-baseline gap-2">
        <h3 className="min-w-0 font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold leading-snug text-paper">{title}</h3>
        {scope && (
          <span className="min-w-0 truncate text-[length:var(--fs-sm)] text-muted" title={scope}>
            {scope}
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-baseline gap-2 text-[length:var(--fs-xs)] text-faint">
          {asOf != null && <span>as of {humanAge(Date.now() - asOf)}</span>}
          <button
            type="button"
            aria-pressed={raw}
            title="Show the tool call and the text the model saw"
            className={`rounded px-1 font-semibold hover:text-paper ${raw ? "text-paper" : ""}`}
            onClick={() => setRaw((r) => !r)}
          >
            raw
          </button>
        </span>
      </header>
      <div className="mt-1">{body}</div>
      {raw && (
        <div className="mt-2 space-y-1.5 border-t border-line-soft pt-2">
          <pre className="overflow-x-auto rounded-lg bg-ink-900 p-2 font-mono text-[length:var(--fs-xs)] text-paper-dim">{JSON.stringify(args, null, 2)}</pre>
          {text && <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-ink-900 p-2 font-mono text-[length:var(--fs-xs)] text-paper-dim">{text}</pre>}
        </div>
      )}
    </section>
  );
}
