import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { tileAt, treemap, type LayoutOptions, type Tile } from "./layout";
import type { Swatch } from "./palette";

export type BlockLabel = { name: string; size?: string };

export type TreemapCanvasProps<T extends { size: number }> = {
  root: T | null;
  kids: (n: T) => readonly T[] | undefined;
  keyOf: (n: T) => string;
  paint: (n: T) => Swatch;
  frame: (n: T) => Swatch;
  label: (n: T) => BlockLabel;
  surface: string;
  selection: string;
  dimmer: string;
  font: string;
  selectedKey?: string | null;
  dim?: (n: T) => boolean;
  onSelect?: (n: T) => void;
  onOpen?: (n: T) => void;
  onKeyDown?: (e: KeyboardEvent<HTMLCanvasElement>) => void;
  onResize?: (w: number, h: number) => void;
  tooltip?: (n: T) => ReactNode;
  layout?: LayoutOptions;
  ariaLabel: string;
  className?: string;
};

function fitText(ctx: CanvasRenderingContext2D, text: string, max: number): string {
  if (max <= 6) return "";
  if (ctx.measureText(text).width <= max) return text;
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (ctx.measureText(`${text.slice(0, mid)}…`).width <= max) lo = mid;
    else hi = mid - 1;
  }
  return lo > 0 ? `${text.slice(0, lo)}…` : "";
}

function patternFor(ctx: CanvasRenderingContext2D, cache: Map<string, CanvasPattern | null>, s: Swatch): CanvasPattern | string {
  if (!s.pattern || !s.mark) return s.fill;
  const key = `${s.pattern}|${s.fill}|${s.mark}`;
  if (!cache.has(key)) {
    const size = s.pattern === "stripes" ? 8 : 6;
    const c = document.createElement("canvas");
    c.width = size;
    c.height = size;
    const g = c.getContext("2d");
    if (g) {
      g.fillStyle = s.fill;
      g.fillRect(0, 0, size, size);
      g.strokeStyle = s.mark;
      g.fillStyle = s.mark;
      if (s.pattern === "stripes") {
        g.lineWidth = 2;
        g.beginPath();
        g.moveTo(-1, size + 1);
        g.lineTo(size + 1, -1);
        g.moveTo(-1, 1);
        g.lineTo(1, -1);
        g.moveTo(size - 1, size + 1);
        g.lineTo(size + 1, size - 1);
        g.stroke();
      } else {
        g.beginPath();
        g.arc(size / 2, size / 2, 1.1, 0, Math.PI * 2);
        g.fill();
      }
    }
    cache.set(key, g ? ctx.createPattern(c, "repeat") : null);
  }
  return cache.get(key) ?? s.fill;
}

export function TreemapCanvas<T extends { size: number }>(p: TreemapCanvasProps<T>) {
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [hover, setHover] = useState<{ tile: Tile<T>; x: number; y: number } | null>(null);
  const { onResize } = p;

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      setSize((s) => (s.w === Math.floor(r.width) && s.h === Math.floor(r.height) ? s : { w: Math.floor(r.width), h: Math.floor(r.height) }));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (size.w > 0 && size.h > 0) onResize?.(size.w, size.h);
  }, [size.w, size.h, onResize]);

  const tiles = useMemo(() => (p.root ? treemap(p.root, p.kids, size.w, size.h, p.layout) : []), [p.root, p.kids, size.w, size.h, p.layout]);

  const { paint, frame, label, keyOf, dim, selectedKey, surface, selection, dimmer, font } = p;
  useEffect(() => {
    const c = canvas.current;
    const ctx = c?.getContext?.("2d");
    if (!c || !ctx) return;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.round(size.w * dpr);
    c.height = Math.round(size.h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = surface;
    ctx.fillRect(0, 0, size.w, size.h);
    const patterns = new Map<string, CanvasPattern | null>();
    ctx.textBaseline = "alphabetic";
    let picked: Tile<T> | null = null;
    for (const t of tiles) {
      if (selectedKey && t.depth > 0 && keyOf(t.node) === selectedKey) picked = t;
      if (t.depth === 0 && t.inner) continue;
      const w = Math.max(1, t.w - 1);
      const h = Math.max(1, t.h - 1);
      const s = t.inner ? frame(t.node) : paint(t.node);
      ctx.fillStyle = patternFor(ctx, patterns, s);
      ctx.fillRect(t.x, t.y, w, h);
      const l = label(t.node);
      ctx.fillStyle = s.ink;
      if (t.inner) {
        if (t.header) {
          ctx.font = `700 11px ${font}`;
          const sizeText = l.size && t.w > 120 ? l.size : "";
          const sizeW = sizeText ? ctx.measureText(sizeText).width + 8 : 0;
          ctx.fillText(fitText(ctx, l.name, t.w - 10 - sizeW), t.x + 4, t.y + 12);
          if (sizeText) {
            ctx.font = `500 10px ${font}`;
            ctx.fillText(sizeText, t.x + t.w - 5 - ctx.measureText(sizeText).width, t.y + 12);
          }
        }
      } else if (t.w >= 30 && t.h >= 15) {
        ctx.font = `700 11px ${font}`;
        const name = fitText(ctx, l.name, t.w - 9);
        if (name) ctx.fillText(name, t.x + 4, t.y + 13);
        if (l.size && t.h >= 29) {
          ctx.font = `500 10px ${font}`;
          ctx.globalAlpha = 0.85;
          ctx.fillText(fitText(ctx, l.size, t.w - 9), t.x + 4, t.y + 26);
          ctx.globalAlpha = 1;
        }
      }
      if (dim?.(t.node)) {
        ctx.fillStyle = dimmer;
        ctx.fillRect(t.x, t.y, w, t.inner ? Math.min(h, t.header ? 16 : 2) : h);
      }
    }
    if (picked) {
      const t = picked;
      ctx.lineWidth = 2;
      ctx.strokeStyle = selection;
      ctx.strokeRect(t.x + 1, t.y + 1, Math.max(1, t.w - 3), Math.max(1, t.h - 3));
      ctx.lineWidth = 1;
      ctx.strokeStyle = surface;
      ctx.strokeRect(t.x + 2.5, t.y + 2.5, Math.max(1, t.w - 6), Math.max(1, t.h - 6));
    }
  }, [tiles, size.w, size.h, paint, frame, label, keyOf, dim, selectedKey, surface, selection, dimmer, font]);

  const at = (e: { clientX: number; clientY: number }) => {
    const r = canvas.current?.getBoundingClientRect();
    if (!r) return null;
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    const t = tileAt(tiles, x, y);
    return t && t.depth === 0 && t.inner ? null : t ? { tile: t, x, y } : null;
  };

  const tip = hover && p.tooltip ? p.tooltip(hover.tile.node) : null;
  return (
    <div ref={box} className={`relative min-h-0 min-w-0 overflow-hidden ${p.className ?? ""}`}>
      <canvas
        ref={canvas}
        {...(p.onKeyDown ? { role: "application", "aria-roledescription": "treemap", tabIndex: 0 } : { role: "img" })}
        aria-label={p.ariaLabel}
        className="absolute inset-0 block h-full w-full cursor-pointer rounded-lg focus-visible:outline focus-visible:outline-2 focus-visible:outline-brass"
        style={{ width: size.w, height: size.h }}
        onMouseMove={(e) => setHover(at(e))}
        onMouseLeave={() => setHover(null)}
        onClick={(e) => {
          const h = at(e);
          if (h) p.onSelect?.(h.tile.node);
        }}
        onDoubleClick={(e) => {
          const h = at(e);
          if (h) p.onOpen?.(h.tile.node);
        }}
        onKeyDown={p.onKeyDown}
      />
      {hover && (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute rounded-[3px] outline outline-2 outline-paper/70"
          style={{ left: hover.tile.x, top: hover.tile.y, width: Math.max(1, hover.tile.w - 1), height: Math.max(1, hover.tile.h - 1) }}
        />
      )}
      {tip && hover && (
        <div
          role="tooltip"
          className="pointer-events-none absolute z-10 max-w-72 rounded-lg border border-line bg-ink-950/95 px-2.5 py-1.5 text-[length:var(--fs-xs)] text-paper shadow-lg"
          style={{
            left: Math.min(hover.x + 14, Math.max(0, size.w - 290)),
            top: hover.y + 16 > size.h - 90 ? Math.max(0, hover.y - 90) : hover.y + 16,
          }}
        >
          {tip}
        </div>
      )}
    </div>
  );
}
