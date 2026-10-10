import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { ApiError, routes } from "../api/client";
import type { ChildRow, IndexMap, MapNode, MapTag, More, TypeRow } from "../api/types";
import { ItemTree, RevealButton, SizeBar, actionBtn } from "../components/ItemTree";
import { digits, humanBytes, samePath } from "../format";
import { STATES, STATE_LABEL, TYPE_SLOTS, useExplorePalette, type ExplorePalette, type Swatch } from "./palette";
import { TreemapCanvas, type BlockLabel } from "./TreemapCanvas";

export type ExploreViewProps = {
  root: string;
  onAsk(text: string): void;
  onCleanUp(path: string): void;
  onWhatIsThis?(path: string): void;
  focus?: string;
};

type Kind = MapNode["kind"] | "more";
type VNode = { key: string; size: number; kind: Kind; node: MapNode; more?: More; children?: VNode[]; parent: VNode | null };
type Selection = { key: string; path: string; name: string; kind: Kind; size: number; files: number; ext?: string; tag?: MapTag; more?: More };
type ColorMode = "knowledge" | "type";

const FOCUS_DEBOUNCE_MS = 600;
const CANVAS_STEP = 64;
const MODE_KEY = "hoardini.explore.color";
const SMALL = "*small";

const lower = (p: string) => p.replace(/\\+$/, "").toLowerCase();
const keyFor = (kind: Kind, path: string) => (kind === "small" || kind === "more" ? `${kind}|${lower(path)}` : lower(path));

function parentPath(p: string): string | null {
  const t = p.replace(/\\+$/, "");
  const i = t.lastIndexOf("\\");
  if (i < 0) return null;
  const up = t.slice(0, i);
  return /^[a-z]:$/i.test(up) ? `${up}\\` : up;
}

function build(n: MapNode, parent: VNode | null, index: Map<string, VNode>): VNode {
  const v: VNode = { key: keyFor(n.kind, n.path), size: n.size, kind: n.kind, node: n, parent };
  index.set(v.key, v);
  if (n.children && n.children.length > 0) {
    v.children = n.children.map((c) => build(c, v, index));
    if (n.more && n.more.bytes > 0) {
      const m: VNode = { key: keyFor("more", n.path), size: n.more.bytes, kind: "more", node: n, more: n.more, parent: v };
      index.set(m.key, m);
      v.children.push(m);
    }
  }
  return v;
}

const kidsOf = (v: VNode) => v.children;
const keyOfV = (v: VNode) => v.key;

function selectionOf(v: VNode): Selection {
  const n = v.node;
  if (v.kind === "more") return { key: v.key, path: n.path, name: `${digits(v.more!.count)} more`, kind: "more", size: v.size, files: v.more!.count, tag: n.tag, more: v.more };
  return { key: v.key, path: n.path, name: v.kind === "small" ? "small files" : n.name, kind: v.kind, size: v.size, files: n.files, ext: n.ext, tag: n.tag };
}

function readMode(): ColorMode {
  try {
    return localStorage.getItem(MODE_KEY) === "type" ? "type" : "knowledge";
  } catch {
    return "knowledge";
  }
}

function saveMode(m: ColorMode) {
  try {
    localStorage.setItem(MODE_KEY, m);
  } catch {
    // storage disabled; the choice lasts for this view only
  }
}

const CATEGORY_LABEL: Record<string, string> = {
  photos: "photos and video",
  documents: "documents",
  "model-weights": "model weights",
};

function typeIdentity(t: TypeRow): string | null {
  if (t.small) return "files under 1 MB, counted with their folder";
  if (t.other) return `${digits(t.other)} more types`;
  return t.label ?? (t.app ? [t.app, t.use].filter(Boolean).join(": ") : null) ?? (t.category ? CATEGORY_LABEL[t.category] ?? t.category : null);
}

const typeName = (t: TypeRow) => (t.small ? "small files" : t.other ? "other types" : t.ext ? `.${t.ext}` : "no extension");
const typeKey = (t: TypeRow) => (t.small ? SMALL : t.other ? null : (t.ext ?? ""));

function Chip({ swatch, children }: { swatch: Swatch; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
      <SwatchBox swatch={swatch} />
      {children}
    </span>
  );
}

function SwatchBox({ swatch, size = 10 }: { swatch: Swatch; size?: number }) {
  const bg =
    swatch.pattern === "stripes" && swatch.mark
      ? `repeating-linear-gradient(135deg, ${swatch.mark} 0 2px, ${swatch.fill} 2px 5px)`
      : swatch.pattern === "dots" && swatch.mark
        ? `radial-gradient(${swatch.mark} 1px, ${swatch.fill} 1.2px) 0 0 / 4px 4px`
        : swatch.fill;
  return <span aria-hidden="true" className="inline-block shrink-0 rounded-[3px] ring-1 ring-line" style={{ width: size, height: size, background: bg }} />;
}

function TagLine({ tag, palette }: { tag?: MapTag; palette: ExplorePalette }) {
  if (!tag) return null;
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5">
      <SwatchBox swatch={palette.states[tag.state]} />
      <span className="font-semibold text-paper">{STATE_LABEL[tag.state]}</span>
      {tag.label && <span className="min-w-0 truncate text-paper-dim">{tag.label}</span>}
      {(tag.verdict === "safe" || tag.verdict === "redownload") && <span className="text-faint">· {tag.verdict}</span>}
    </span>
  );
}

const btn =
  "inline-flex items-center gap-1 whitespace-nowrap rounded-full border border-line px-2.5 py-0.5 text-[length:var(--fs-xs)] font-bold text-paper-dim hover:border-brass/60 hover:text-brass focus-visible:outline focus-visible:outline-1 focus-visible:outline-brass disabled:opacity-40";
const primary =
  "inline-flex items-center whitespace-nowrap rounded-full bg-brass px-2.5 py-0.5 text-[length:var(--fs-xs)] font-bold text-on-accent hover:brightness-110 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-brass";

export function ExploreView({ root, onAsk, onCleanUp, onWhatIsThis, focus }: ExploreViewProps) {
  const { palette, font } = useExplorePalette();
  const [view, setView] = useState(focus ?? root);
  const [dims, setDims] = useState<{ w: number; h: number } | null>(null);
  const [data, setData] = useState<{ map?: IndexMap; err?: string; loading: boolean }>({ loading: true });
  const [selected, setSelected] = useState<Selection | null>(null);
  const [mode, setMode] = useState<ColorMode>(readMode);
  const [typeSel, setTypeSel] = useState<string | null>(null);
  const [types, setTypes] = useState<{ path: string; rows?: TypeRow[]; err?: string } | null>(null);
  const [rootTypes, setRootTypes] = useState<TypeRow[]>([]);
  const want = useRef<string | null>(focus ?? null);
  const [retry, setRetry] = useState(0);

  useEffect(() => {
    want.current = null;
    setView(root);
  }, [root]);

  useEffect(() => {
    if (!focus) return;
    want.current = focus;
    setView(focus);
  }, [focus]);

  const zoom = useCallback((path: string, select?: string) => {
    want.current = select ?? path;
    setView(path);
  }, []);

  useEffect(() => {
    if (!dims) return;
    let live = true;
    setData((d) => ({ ...d, loading: true, err: undefined }));
    routes
      .map(view, dims.w, dims.h)
      .then((map) => live && setData({ map, loading: false }))
      .catch((e) => {
        if (!live) return;
        const up = parentPath(view);
        if (e instanceof ApiError && e.status === 400 && up) {
          want.current = view;
          setView(up);
          return;
        }
        setData({ loading: false, err: e instanceof Error ? e.message : String(e) });
      });
    return () => {
      live = false;
    };
  }, [view, dims, retry]);

  useEffect(() => {
    let live = true;
    routes
      .types(root)
      .then((rows) => live && setRootTypes(rows))
      .catch(() => live && setRootTypes([]));
    return () => {
      live = false;
    };
  }, [root]);

  const map = data.map;
  // Types follow the folder the map resolved to, so a file focus never asks for a file's types.
  const shown = map?.tree.path ?? null;
  useEffect(() => {
    if (!shown) return;
    let live = true;
    routes
      .types(shown)
      .then((rows) => live && setTypes({ path: shown, rows }))
      .catch((e) => live && setTypes({ path: shown, err: e instanceof Error ? e.message : String(e) }));
    return () => {
      live = false;
    };
  }, [shown]);

  const { vroot, index } = useMemo(() => {
    const index = new Map<string, VNode>();
    const vroot = map ? build(map.tree, null, index) : null;
    return { vroot, index };
  }, [map]);

  const tagOf = useCallback((path: string): MapTag | undefined => index.get(lower(path))?.node.tag, [index]);

  // A zoom or an outside focus names what to select once the new map is in.
  useEffect(() => {
    if (!vroot) return;
    const target = want.current;
    want.current = null;
    if (target) {
      let p: string | null = target;
      while (p) {
        const v = index.get(lower(p));
        if (v) {
          setSelected(selectionOf(v));
          return;
        }
        p = parentPath(p);
      }
      setSelected(selectionOf(vroot));
      return;
    }
    setSelected((s) => {
      if (!s) return selectionOf(vroot);
      const v = index.get(s.key);
      return v ? selectionOf(v) : selectionOf(vroot);
    });
  }, [vroot, index]);

  const focusPath = selected?.path ?? view;
  useEffect(() => {
    const t = setTimeout(() => void routes.exploreFocus(focusPath).catch(() => {}), FOCUS_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [focusPath]);
  useEffect(() => () => void routes.exploreFocus(null).catch(() => {}), []);

  const onResize = useCallback((w: number, h: number) => {
    const step = (n: number) => Math.max(CANVAS_STEP, Math.round(n / CANVAS_STEP) * CANVAS_STEP);
    setDims((d) => (d && d.w === step(w) && d.h === step(h) ? d : { w: step(w), h: step(h) }));
  }, []);

  const slots = useMemo(() => {
    const m = new Map<string, number>();
    for (const t of rootTypes) {
      if (m.size >= TYPE_SLOTS) break;
      if (t.ext != null && !t.small && !t.other) m.set(t.ext, m.size);
    }
    return m;
  }, [rootTypes]);
  const typeSwatch = useCallback((ext: string | null | undefined): Swatch => {
    const i = ext == null ? undefined : slots.get(ext);
    return i == null ? palette.other : palette.types[i]!;
  }, [slots, palette]);

  const paint = useCallback(
    (v: VNode): Swatch => {
      if (v.kind === "more") return palette.more;
      if (mode === "type") return v.kind === "file" ? typeSwatch(v.node.ext) : v.kind === "small" ? palette.more : palette.other;
      return palette.states[v.node.tag.state];
    },
    [palette, mode, typeSwatch],
  );
  const frame = useCallback((v: VNode): Swatch => (mode === "type" ? palette.frame : palette.frames[v.node.tag.state]), [palette, mode]);
  const label = useCallback((v: VNode): BlockLabel => {
    if (v.kind === "more") return { name: `${digits(v.more!.count)} more`, size: humanBytes(v.size) };
    return { name: v.kind === "small" ? "small files" : v.node.name, size: humanBytes(v.size) };
  }, []);
  const dim = useMemo(
    () => (typeSel == null ? undefined : (v: VNode) => (typeSel === SMALL ? v.kind !== "small" : !(v.kind === "file" && v.node.ext === typeSel))),
    [typeSel],
  );

  const total = map?.tree.size ?? 0;
  const pct = useCallback(
    (n: number) => (total > 0 ? (n / total < 0.001 ? "<0.1%" : `${((n / total) * 100).toFixed(n / total < 0.1 ? 1 : 0)}%`) : ""),
    [total],
  );
  const tooltip = useCallback(
    (v: VNode) => (
      <>
        <div className="truncate font-bold">{label(v).name}</div>
        <div className="truncate text-faint">{v.node.path}</div>
        <div className="mt-0.5 tabular-nums text-paper-dim">
          {humanBytes(v.size)} · {pct(v.size)}
          {v.kind === "dir" || v.kind === "small" ? ` · ${digits(v.node.files)} files` : ""}
        </div>
        {v.kind !== "more" && (
          <div className="mt-0.5">
            <TagLine tag={v.node.tag} palette={palette} />
          </div>
        )}
      </>
    ),
    [label, palette, pct],
  );

  const select = useCallback((v: VNode) => setSelected(selectionOf(v)), []);
  const open = useCallback(
    (v: VNode) => {
      if (v.kind === "dir" && v !== vroot) zoom(v.node.path);
      else if (v.kind === "more" && v.parent && v.parent !== vroot) zoom(v.node.path);
      else select(v);
    },
    [vroot, zoom, select],
  );

  const up = map?.ancestors[map.ancestors.length - 1]?.path ?? null;
  const zoomOut = useCallback(() => {
    if (up) zoom(up, view);
  }, [up, view, zoom]);

  const here = map?.tree.path ?? view;
  const canZoomIn = (s: Selection | null) => !!s && (s.kind === "dir" || s.kind === "more") && !samePath(s.path, here);

  const onKey = (e: KeyboardEvent<HTMLCanvasElement>) => {
    if (!vroot) return;
    const cur = selected ? index.get(selected.key) : undefined;
    if (e.key === "Enter" && cur) {
      e.preventDefault();
      open(cur);
    } else if ((e.key === "Backspace" || e.key === "Escape") && up) {
      e.preventDefault();
      zoomOut();
    } else if (e.key.startsWith("Arrow")) {
      e.preventDefault();
      const sibs = cur?.parent?.children ?? vroot.children ?? [];
      if (sibs.length === 0) return;
      if (!cur || cur === vroot) return select(sibs[0]!);
      const i = sibs.indexOf(cur);
      if (e.key === "ArrowLeft" && e.altKey) return cur.parent && cur.parent !== vroot ? select(cur.parent) : undefined;
      if (e.key === "ArrowRight" && e.altKey) return cur.children?.[0] ? select(cur.children[0]) : undefined;
      const next = e.key === "ArrowDown" || e.key === "ArrowRight" ? i + 1 : i - 1;
      select(sibs[(next + sibs.length) % sibs.length]!);
    }
  };

  const pickRow = useCallback(
    (r: ChildRow) => {
      const v = index.get(lower(r.path));
      setSelected(v ? selectionOf(v) : { key: lower(r.path), path: r.path, name: r.name, kind: r.kind, size: r.size, files: r.files, tag: tagOf(r.path) });
    },
    [index, tagOf],
  );

  const setColor = (m: ColorMode) => {
    setMode(m);
    saveMode(m);
  };

  const crumbs = map ? [...map.ancestors, { path: map.tree.path, name: map.tree.name }] : [];
  const trail = useMemo(() => {
    const out: string[] = [];
    for (let v = selected ? index.get(selected.key)?.parent : undefined; v; v = v.parent) out.push(v.node.path);
    return out;
  }, [selected, index]);
  const sel = selected;
  const selTag = sel?.tag ?? (sel ? tagOf(sel.path) : undefined);
  const typeRows = types?.path === shown ? types.rows : undefined;
  const typeTotal = typeRows?.reduce((a, t) => a + t.bytes, 0) ?? 0;
  const selType = sel?.kind === "file" ? (sel.ext ?? "") : sel?.kind === "small" ? SMALL : null;

  return (
    <div className="@container flex h-full min-h-0 w-full flex-col gap-2 overflow-y-auto p-3 @3xl:overflow-hidden" aria-label="Explore">
      <header className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <h2 className="font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold text-paper">Explore</h2>
        <nav aria-label="Folder path" className="flex min-w-0 flex-1 flex-wrap items-center gap-0.5 text-[length:var(--fs-sm)]">
          <button type="button" className={`${actionBtn} mr-1`} disabled={!up} onClick={zoomOut} title="Zoom out to the folder above" aria-label="Zoom out">
            ↑ Up
          </button>
          {crumbs.map((c, i) => (
            <span key={c.path} className="inline-flex min-w-0 items-center">
              {i > 0 && <span className="px-0.5 text-faint">›</span>}
              {i === crumbs.length - 1 ? (
                <span className="truncate font-bold text-paper" aria-current="location" title={c.path}>
                  {c.name}
                </span>
              ) : (
                <button type="button" className="truncate rounded px-1 font-semibold text-muted hover:bg-ink-800 hover:text-paper" title={c.path} onClick={() => zoom(c.path, view)}>
                  {c.name}
                </button>
              )}
            </span>
          ))}
        </nav>
        <div role="radiogroup" aria-label="Color blocks by" className="flex rounded-full border border-line bg-ink-850 p-0.5 text-[length:var(--fs-xs)] font-bold">
          {(["knowledge", "type"] as const).map((m) => (
            <button
              key={m}
              type="button"
              role="radio"
              aria-checked={mode === m}
              className={`rounded-full px-2.5 py-0.5 ${mode === m ? "bg-brass text-on-accent" : "text-muted hover:text-paper"}`}
              onClick={() => setColor(m)}
            >
              {m === "knowledge" ? "What Hoardini knows" : "File type"}
            </button>
          ))}
        </div>
      </header>

      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[length:var(--fs-xs)] text-muted">
        {mode === "knowledge"
          ? STATES.map((s) => (
              <Chip key={s} swatch={palette.states[s]}>
                {STATE_LABEL[s]}
              </Chip>
            ))
          : [...slots.entries()].map(([ext, i]) => (
              <Chip key={ext} swatch={palette.types[i]!}>
                .{ext || "(none)"}
              </Chip>
            ))}
        {mode === "type" && <Chip swatch={palette.other}>other</Chip>}
        <Chip swatch={palette.more}>merged</Chip>
        {map && (
          <span className="ml-auto tabular-nums text-faint">
            {humanBytes(map.tree.size)} · {map.capped ? `the ${digits(map.nodes)} biggest blocks` : `${digits(map.nodes)} blocks`}, smaller than {humanBytes(map.min_bytes)} merged
          </span>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-xl border border-line bg-ink-850 px-3 py-2" aria-live="polite">
        {sel ? (
          <>
            <div className="min-w-0 flex-[1_1_16rem]">
              <div className="flex min-w-0 items-baseline gap-2">
                <span className="truncate font-bold text-paper" title={sel.path}>
                  {sel.name}
                </span>
                <span className="min-w-0 truncate text-[length:var(--fs-xs)] text-faint" title={sel.path}>
                  {sel.kind === "more" ? `in ${sel.path}` : sel.path}
                </span>
              </div>
              <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 text-[length:var(--fs-xs)] text-muted">
                <span className="font-bold tabular-nums text-paper">{humanBytes(sel.size)}</span>
                <span className="tabular-nums">{pct(sel.size)} of this view</span>
                {(sel.kind === "dir" || sel.kind === "small") && <span className="tabular-nums">{digits(sel.files)} files</span>}
                {sel.kind !== "more" && <TagLine tag={selTag} palette={palette} />}
              </div>
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              {sel.kind === "dir" && (
                <button type="button" className={primary} onClick={() => onCleanUp(sel.path)}>
                  Clean up
                </button>
              )}
              {onWhatIsThis && sel.kind !== "more" && sel.kind !== "small" && (
                <button type="button" className={btn} onClick={() => onWhatIsThis(sel.path)}>
                  What is this?
                </button>
              )}
              {sel.kind !== "more" && (
                <button type="button" className={btn} onClick={() => onAsk(`What is ${sel.path}?`)}>
                  Ask Hoardini
                </button>
              )}
              {sel.kind !== "more" && (
                <span className="inline-flex items-center rounded-full border border-line px-1">
                  <RevealButton path={sel.path} label="Show in Explorer" />
                </span>
              )}
              {canZoomIn(sel) && (
                <button type="button" className={btn} onClick={() => zoom(sel.path)}>
                  Zoom in
                </button>
              )}
            </div>
          </>
        ) : (
          <span className="text-[length:var(--fs-sm)] text-muted">Click a block, a folder or a type.</span>
        )}
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-1 gap-2 @3xl:grid-cols-[minmax(0,1fr)_17rem] @3xl:grid-rows-[minmax(6rem,32%)_minmax(0,1fr)]">
        <section aria-label="Treemap" className="relative order-1 h-[22rem] min-h-0 rounded-xl border border-line bg-ink-900 p-1 @3xl:order-none @3xl:col-start-1 @3xl:row-start-2 @3xl:h-auto">
          <TreemapCanvas
            root={vroot}
            kids={kidsOf}
            keyOf={keyOfV}
            paint={paint}
            frame={frame}
            label={label}
            surface={palette.surface}
            selection={palette.selection}
            dimmer={palette.dimmer}
            font={font}
            selectedKey={sel?.key ?? null}
            dim={dim}
            onSelect={select}
            onOpen={open}
            onKeyDown={onKey}
            onResize={onResize}
            tooltip={tooltip}
            ariaLabel={`Treemap of ${view}: click a block to select it, double-click or Enter to zoom in, Backspace to zoom out, arrow keys to move between neighbors.`}
            className="h-full w-full"
          />
          {data.loading && (
            <div className="pointer-events-none absolute inset-0 grid place-items-center">
              <span className="pulse-soft rounded-full bg-ink-950/80 px-3 py-1 text-[length:var(--fs-xs)] font-semibold text-muted">Loading...</span>
            </div>
          )}
          {data.err && (
            <div className="absolute inset-0 grid place-items-center p-4 text-center text-[length:var(--fs-sm)] text-err">
              <div>
                Could not draw this folder: {data.err}
                <div className="mt-2">
                  <button type="button" className={btn} onClick={() => setRetry((r) => r + 1)}>
                    Try again
                  </button>
                </div>
              </div>
            </div>
          )}
        </section>

        <section aria-label="Folders" className="@container order-2 max-h-72 min-h-0 overflow-auto rounded-xl border border-line bg-ink-900 px-2 py-1.5 @3xl:order-none @3xl:col-start-1 @3xl:row-start-1 @3xl:max-h-none">
          <div className="label px-1 text-[length:var(--fs-xs)] uppercase">Folders</div>
          <ItemTree
            key={view}
            path={view}
            selected={sel?.path ?? null}
            trail={trail}
            onSelect={pickRow}
            onOpen={(r) => r.kind === "dir" && zoom(r.path)}
            actions={(r) =>
              r.kind === "dir" ? (
                <button type="button" className={actionBtn} onClick={() => zoom(r.path)} title={`Zoom into ${r.path}`}>
                  Zoom in
                </button>
              ) : null
            }
          />
        </section>

        <section aria-label="By type" className="order-3 max-h-80 min-h-0 overflow-auto rounded-xl border border-line bg-ink-900 px-2 py-1.5 @3xl:order-none @3xl:col-start-2 @3xl:row-span-2 @3xl:row-start-1 @3xl:max-h-none">
          <div className="label flex items-baseline px-1 text-[length:var(--fs-xs)] uppercase">
            By type
            {typeSel != null && (
              <button type="button" className={`${actionBtn} ml-auto normal-case`} onClick={() => setTypeSel(null)}>
                Clear
              </button>
            )}
          </div>
          {types?.path === shown && types.err && <p className="px-1 py-1 text-[length:var(--fs-xs)] text-err">Could not list types: {types.err}</p>}
          {!typeRows && !(types?.path === shown && types.err) && <p className="pulse-soft px-1 py-1 text-[length:var(--fs-xs)] text-faint">Loading...</p>}
          <ul className="mt-1 space-y-0.5">
            {typeRows?.map((t) => {
              const k = typeKey(t);
              const on = k != null && typeSel === k;
              const current = k != null && selType === k;
              const id = typeIdentity(t);
              const sw = t.small ? palette.more : typeSwatch(t.ext);
              return (
                <li key={`${t.ext}|${t.small ? "s" : t.other ? "o" : ""}`}>
                  <div
                    className={`group relative rounded-lg px-1.5 py-1 ${on ? "bg-brass/12 ring-1 ring-brass/50" : current ? "bg-ink-800" : "hover:bg-ink-800"}`}
                    aria-current={current || undefined}
                  >
                    <button
                      type="button"
                      className="flex w-full items-center gap-2 text-left text-[length:var(--fs-sm)]"
                      aria-pressed={on}
                      disabled={k == null}
                      title={k == null ? undefined : on ? "Show every block again" : `Highlight ${typeName(t)} in the map`}
                      onClick={() => setTypeSel(on ? null : k)}
                    >
                      <SwatchBox swatch={sw} />
                      <span className="min-w-0 flex-1 truncate font-mono text-[length:var(--fs-xs)] font-bold text-paper">{typeName(t)}</span>
                      <span className="shrink-0 tabular-nums text-paper">{humanBytes(t.bytes)}</span>
                    </button>
                    <div className="mt-0.5 flex items-center gap-2 pl-[18px]">
                      <span className="min-w-0 flex-1 truncate text-[length:var(--fs-xs)] text-faint" title={id ?? undefined}>
                        {id ?? `${digits(t.files)} files`}
                      </span>
                      {!id && t.ext && (
                        <button type="button" className={`${actionBtn} -my-0.5`} onClick={() => onAsk(`What are .${t.ext} files?`)}>
                          What is this?
                        </button>
                      )}
                      <span className="w-14 shrink-0">
                        <SizeBar fraction={typeTotal > 0 ? t.bytes / typeTotal : 0} />
                      </span>
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      </div>
    </div>
  );
}
