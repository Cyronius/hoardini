import { useEffect, useState } from "react";
import type { TagState } from "../api/types";
import { contrast, themeDef, DEFAULT_LIGHT, type ThemeTokens } from "../theme";

export type Pattern = "stripes" | "dots";
export type Swatch = { fill: string; ink: string; pattern?: Pattern; mark?: string };

export type ExplorePalette = {
  dark: boolean;
  surface: string;
  selection: string;
  dimmer: string;
  frame: Swatch;
  states: Record<TagState, Swatch>;
  frames: Record<TagState, Swatch>;
  more: Swatch;
  plain: Swatch;
};

export const STATES: TagState[] = ["proposed", "review", "keep", "system", "user", "unknown"];
export const STATE_LABEL: Record<TagState, string> = {
  proposed: "Proposed",
  review: "Your call",
  keep: "Keep",
  system: "System",
  user: "Your data",
  unknown: "Unknown",
};

function rgb(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const h = m[1]!.length === 3 ? [...m[1]!].map((c) => c + c).join("") : m[1]!;
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

const hex2 = (n: number) => Math.round(Math.max(0, Math.min(255, n))).toString(16).padStart(2, "0");

export function mix(a: string, b: string, t: number): string {
  const x = rgb(a) ?? [0, 0, 0];
  const y = rgb(b) ?? [0, 0, 0];
  return `#${hex2(x[0] * t + y[0] * (1 - t))}${hex2(x[1] * t + y[1] * (1 - t))}${hex2(x[2] * t + y[2] * (1 - t))}`;
}

export function alpha(hex: string, a: number): string {
  const c = rgb(hex) ?? [0, 0, 0];
  return `rgba(${c[0]}, ${c[1]}, ${c[2]}, ${a})`;
}

// The theme's own ink when it reads at 4.5:1; black or white always does, so they are the fallback.
export function inkOn(fill: string, t: ThemeTokens): string {
  const own = contrast(t.text, fill) >= contrast(t.bg0, fill) ? t.text : t.bg0;
  if (contrast(own, fill) >= 4.5) return own;
  return contrast("#000000", fill) >= contrast("#ffffff", fill) ? "#000000" : "#ffffff";
}

export function explorePalette(t: ThemeTokens): ExplorePalette {
  const dark = contrast(t.bg1, "#ffffff") > contrast(t.bg1, "#000000");
  const surface = t.bg1;
  const sw = (fill: string, extra: Partial<Swatch> = {}): Swatch => ({ fill, ink: inkOn(fill, t), ...extra });
  const base: Record<TagState, string> = {
    proposed: t.ok,
    review: dark ? "#e0a83a" : "#c98a12",
    keep: mix(t.faint, surface, 0.55),
    system: mix(t.muted, surface, 0.8),
    user: t.info,
    unknown: t.bg3,
  };
  const states = {} as Record<TagState, Swatch>;
  const frames = {} as Record<TagState, Swatch>;
  for (const s of STATES) {
    const fill = s === "unknown" || s === "keep" || s === "system" ? base[s] : mix(base[s], surface, 0.8);
    states[s] = s === "unknown" ? sw(fill, { pattern: "stripes", mark: mix(t.faint, fill, 0.5) }) : sw(fill);
    frames[s] = sw(mix(s === "unknown" ? t.faint : base[s], t.bg2, 0.22));
  }
  const more = t.bg2;
  return {
    dark,
    surface,
    selection: t.text,
    dimmer: alpha(surface, 0.74),
    frame: sw(t.bg2),
    states,
    frames,
    more: sw(more, { pattern: "dots", mark: mix(t.faint, more, 0.6) }),
    plain: sw(mix(t.accent, surface, 0.3)),
  };
}

const VARS: Record<keyof ThemeTokens, string> = {
  bg0: "--color-ink-950",
  bg1: "--color-ink-900",
  bg2: "--color-ink-850",
  bg3: "--color-ink-800",
  line: "--color-line",
  lineSoft: "--color-line-soft",
  text: "--color-paper",
  textDim: "--color-paper-dim",
  muted: "--color-muted",
  faint: "--color-faint",
  accent: "--color-brass",
  ok: "--color-ok",
  err: "--color-err",
  info: "--color-info",
};

// Canvas cannot use CSS variables, so the theme's tokens are read back from the document.
export function tokensFromCss(el: Element = document.documentElement): ThemeTokens {
  const css = getComputedStyle(el);
  const fallback = themeDef(DEFAULT_LIGHT).tokens;
  const out = { ...fallback };
  for (const k of Object.keys(VARS) as (keyof ThemeTokens)[]) {
    const v = css.getPropertyValue(VARS[k]).trim();
    if (rgb(v)) out[k] = v;
  }
  return out;
}

export function fontFromCss(el: Element = document.documentElement): string {
  return getComputedStyle(el).getPropertyValue("--font-sans").trim() || "sans-serif";
}

export function useExplorePalette(): { palette: ExplorePalette; font: string } {
  const read = () => ({ palette: explorePalette(tokensFromCss()), font: fontFromCss() });
  const [p, setP] = useState(read);
  useEffect(() => {
    const obs = new MutationObserver(() => setP(read()));
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["style", "data-theme"] });
    return () => obs.disconnect();
  }, []);
  return p;
}
