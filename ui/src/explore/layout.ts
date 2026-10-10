export type Rect = { x: number; y: number; w: number; h: number };

export type Tile<T> = Rect & {
  node: T;
  depth: number;
  parent: T | null;
  header: boolean;
  inner: Rect | null;
};

export type LayoutOptions = {
  header?: number;
  pad?: number;
  minHeaderW?: number;
  minInner?: number;
};

const DEFAULTS: Required<LayoutOptions> = { header: 16, pad: 2, minHeaderW: 44, minInner: 10 };

type Edge = { x0: number; y0: number; x1: number; y1: number };

function worst(areas: number[], side: number): number {
  let sum = 0;
  let max = 0;
  let min = Infinity;
  for (const a of areas) {
    sum += a;
    if (a > max) max = a;
    if (a < min) min = a;
  }
  const s2 = side * side;
  const t2 = sum * sum;
  return Math.max((s2 * max) / t2, t2 / (s2 * min));
}

// Edges round from shared float values, so neighbors meet exactly and the parent is tiled with no gap.
export function squarify(sizes: readonly number[], rect: Rect): Rect[] {
  const out: Rect[] = sizes.map(() => ({ x: rect.x, y: rect.y, w: 0, h: 0 }));
  const order = sizes
    .map((size, i) => ({ size, i }))
    .filter((e) => e.size > 0)
    .sort((a, b) => b.size - a.size || a.i - b.i);
  const total = order.reduce((s, e) => s + e.size, 0);
  if (total <= 0 || rect.w <= 0 || rect.h <= 0) return out;
  const scale = (rect.w * rect.h) / total;
  const areas = order.map((e) => e.size * scale);
  const edges: Edge[] = new Array(order.length);
  let x = rect.x;
  let y = rect.y;
  const right = rect.x + rect.w;
  const bottom = rect.y + rect.h;
  let i = 0;
  while (i < order.length) {
    const w = right - x;
    const h = bottom - y;
    const side = Math.min(w, h);
    const row = [areas[i]!];
    let j = i + 1;
    while (j < order.length && worst([...row, areas[j]!], side) <= worst(row, side)) {
      row.push(areas[j]!);
      j++;
    }
    const rowArea = row.reduce((s, a) => s + a, 0);
    const last = j === order.length;
    if (w >= h) {
      const x1 = last ? right : Math.min(right, x + rowArea / h);
      let yy = y;
      for (let k = i; k < j; k++) {
        const y1 = k === j - 1 ? bottom : Math.min(bottom, yy + areas[k]! / Math.max(x1 - x, 1e-9));
        edges[k] = { x0: x, y0: yy, x1, y1 };
        yy = y1;
      }
      x = x1;
    } else {
      const y1 = last ? bottom : Math.min(bottom, y + rowArea / w);
      let xx = x;
      for (let k = i; k < j; k++) {
        const x1 = k === j - 1 ? right : Math.min(right, xx + areas[k]! / Math.max(y1 - y, 1e-9));
        edges[k] = { x0: xx, y0: y, x1, y1 };
        xx = x1;
      }
      y = y1;
    }
    i = j;
  }
  order.forEach((e, k) => {
    const g = edges[k]!;
    const x0 = Math.round(g.x0);
    const y0 = Math.round(g.y0);
    out[e.i] = { x: x0, y: y0, w: Math.round(g.x1) - x0, h: Math.round(g.y1) - y0 };
  });
  return out;
}

export function treemap<T extends { size: number }>(
  root: T,
  kids: (n: T) => readonly T[] | undefined,
  width: number,
  height: number,
  options: LayoutOptions = {},
): Tile<T>[] {
  const o = { ...DEFAULTS, ...options };
  const tiles: Tile<T>[] = [];
  const place = (node: T, r: Rect, depth: number, parent: T | null) => {
    const children = (kids(node) ?? []).filter((c) => c.size > 0);
    const tile: Tile<T> = { ...r, node, depth, parent, header: false, inner: null };
    tiles.push(tile);
    if (children.length === 0) return;
    let inner: Rect = r;
    if (depth > 0) {
      const header = r.w >= o.minHeaderW && r.h >= o.header + o.pad + o.minInner;
      const pad = r.w > 2 * o.pad + o.minInner && r.h > 2 * o.pad + o.minInner ? o.pad : 0;
      const top = header ? o.header : pad;
      inner = { x: r.x + pad, y: r.y + top, w: r.w - 2 * pad, h: r.h - top - pad };
      tile.header = header;
    }
    if (inner.w <= 0 || inner.h <= 0) return;
    tile.inner = inner;
    const rects = squarify(
      children.map((c) => c.size),
      inner,
    );
    children.forEach((c, i) => {
      const cr = rects[i]!;
      if (cr.w > 0 && cr.h > 0) place(c, cr, depth + 1, node);
    });
  };
  if (width > 0 && height > 0) place(root, { x: 0, y: 0, w: Math.floor(width), h: Math.floor(height) }, 0, null);
  return tiles;
}

export function tileAt<T>(tiles: readonly Tile<T>[], x: number, y: number): Tile<T> | null {
  for (let i = tiles.length - 1; i >= 0; i--) {
    const t = tiles[i]!;
    if (x >= t.x && x < t.x + t.w && y >= t.y && y < t.y + t.h) return t;
  }
  return null;
}
