// Traces: UI-TREEMAP-LAYOUT
import { describe, expect, it } from "vitest";
import { squarify, tileAt, treemap, type Rect, type Tile } from "@ui/explore/layout";

type N = { name: string; size: number; children?: N[] };
const kids = (n: N) => n.children;

const area = (r: Rect) => r.w * r.h;
const overlap = (a: Rect, b: Rect) => Math.max(0, Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)) * Math.max(0, Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y));
const inside = (r: Rect, o: Rect) => r.x >= o.x && r.y >= o.y && r.x + r.w <= o.x + o.w && r.y + r.h <= o.y + o.h;

function tiles(rects: Rect[], parent: Rect) {
  expect(rects.reduce((a, r) => a + area(r), 0)).toBe(area(parent));
  rects.forEach((r, i) => {
    expect(inside(r, parent)).toBe(true);
    rects.slice(i + 1).forEach((o) => expect(overlap(r, o)).toBe(0));
  });
}

describe("UI-TREEMAP-LAYOUT", () => {
  it("squarifies the classic example into exact whole-pixel rectangles", () => {
    const rects = squarify([6, 6, 4, 3, 2, 2, 1], { x: 0, y: 0, w: 600, h: 400 });
    expect(rects).toEqual([
      { x: 0, y: 0, w: 300, h: 200 },
      { x: 0, y: 200, w: 300, h: 200 },
      { x: 300, y: 0, w: 171, h: 233 },
      { x: 471, y: 0, w: 129, h: 233 },
      { x: 300, y: 233, w: 120, h: 167 },
      { x: 420, y: 233, w: 120, h: 167 },
      { x: 540, y: 233, w: 60, h: 167 },
    ]);
  });

  it("tiles the parent exactly with areas in proportion to size", () => {
    const sizes = [97, 61, 40, 33, 33, 12, 8, 5, 2, 1, 1, 0.5];
    const parent = { x: 7, y: 3, w: 413, h: 287 };
    const rects = squarify(sizes, parent);
    tiles(rects, parent);
    const total = sizes.reduce((a, s) => a + s, 0);
    rects.forEach((r, i) => {
      const exact = (sizes[i]! / total) * area(parent);
      expect(Math.abs(area(r) - exact)).toBeLessThanOrEqual(r.w + r.h + 1);
    });
  });

  it("keeps input order for equal sizes and gives empty items no area", () => {
    const rects = squarify([5, 0, 5], { x: 0, y: 0, w: 100, h: 50 });
    expect(rects[0]).toEqual({ x: 0, y: 0, w: 50, h: 50 });
    expect(rects[2]).toEqual({ x: 50, y: 0, w: 50, h: 50 });
    expect(area(rects[1]!)).toBe(0);
    expect(squarify([5, 0, 5], { x: 0, y: 0, w: 100, h: 50 })).toEqual(rects);
  });

  it("nests folders under a name strip and tiles each folder's padded inside", () => {
    const root: N = {
      name: "C:",
      size: 100,
      children: [
        { name: "Users", size: 60, children: [{ name: "a", size: 40 }, { name: "b", size: 15, children: [{ name: "b1", size: 10 }, { name: "b2", size: 5 }] }, { name: "c", size: 5 }] },
        { name: "code", size: 30, children: [{ name: "x", size: 30 }] },
        { name: "tiny", size: 0.3, children: [{ name: "t", size: 0.3 }] },
        { name: "gone", size: 0 },
        { name: "Windows", size: 9.7 },
      ],
    };
    const all = treemap(root, kids, 640, 400);
    const byName = new Map(all.map((t) => [t.node.name, t]));
    expect(all[0]).toMatchObject({ x: 0, y: 0, w: 640, h: 400, depth: 0, header: false, inner: { x: 0, y: 0, w: 640, h: 400 } });
    expect(byName.has("gone")).toBe(false);
    const users = byName.get("Users")!;
    expect(users.header).toBe(true);
    expect(users.inner).toEqual({ x: users.x + 2, y: users.y + 16, w: users.w - 4, h: users.h - 18 });
    expect(byName.get("Windows")!.inner).toBeNull();
    const tiny = byName.get("tiny")!;
    expect(tiny.w < 44 || tiny.h < 28).toBe(true);
    expect(tiny.header).toBe(false);
    for (const t of all.filter((x) => x.inner)) {
      const children = all.filter((c) => c.parent === t.node);
      tiles(children, t.inner!);
      expect(children.every((c) => c.depth === t.depth + 1)).toBe(true);
    }
    expect(treemap(root, kids, 640, 400)).toEqual(all);
  });

  it("finds the deepest block under a point", () => {
    const root: N = { name: "r", size: 2, children: [{ name: "a", size: 1, children: [{ name: "a1", size: 1 }] }, { name: "b", size: 1 }] };
    const all: Tile<N>[] = treemap(root, kids, 200, 100);
    const a = all.find((t) => t.node.name === "a")!;
    expect(tileAt(all, a.x + 5, a.y + 5)?.node.name).toBe("a");
    expect(tileAt(all, a.inner!.x + 5, a.inner!.y + 5)?.node.name).toBe("a1");
    expect(tileAt(all, 199, 99)?.node.name).toBe("b");
    expect(tileAt(all, 500, 500)).toBeNull();
  });
});
