import type { Status } from "../../api/types";

export type Volume = Status["volumes"][number];

export const GIRTH_MIN = 9;
export const GIRTH_MAX = 26;

export function girthFor(used: number): number {
  const u = Math.min(1, Math.max(0, used));
  // Mostly flat until ~70% used, then steep, so the last stretch is dramatic.
  return GIRTH_MIN + (GIRTH_MAX - GIRTH_MIN) * (0.35 * u + 0.65 * u ** 4);
}

export function selectedVolume(volumes: Volume[] | undefined): Volume | undefined {
  return volumes?.find((v) => v.selected) ?? volumes?.find((v) => v.root.toUpperCase().startsWith("C:")) ?? volumes?.[0];
}

export function usedFraction(v: Volume | undefined): number | null {
  return v && v.total_bytes > 0 ? Math.min(1, Math.max(0, 1 - v.free_bytes / v.total_bytes)) : null;
}
