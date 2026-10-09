export type ThemeMode = "dark" | "light";

export interface ThemeTokens {
  bg0: string;
  bg1: string;
  bg2: string;
  bg3: string;
  line: string;
  lineSoft: string;
  text: string;
  textDim: string;
  muted: string;
  faint: string;
  accent: string;
  ok: string;
  err: string;
  info: string;
}

export interface ThemeDef {
  id: string;
  label: string;
  mode: ThemeMode;
  fonts: [string, string, string];
  tokens: ThemeTokens;
}

const DISPLAY = "'Fraunces Variable', Georgia, serif";
const SANS = "'Nunito Variable', system-ui, sans-serif";
const MONO = "'JetBrains Mono Variable', ui-monospace, Consolas, monospace";
const ROUND: ThemeDef["fonts"] = [SANS, SANS, MONO];
const SERIFY: ThemeDef["fonts"] = [DISPLAY, SANS, MONO];

export const THEMES: ThemeDef[] = [
  { id: "marshmallow", label: "Marshmallow", mode: "light", fonts: SERIFY,
    tokens: { bg0: "#f6ece2", bg1: "#fff8f0", bg2: "#fbeee1", bg3: "#f2dfcf", line: "#e8d3c0", lineSoft: "#f0e1d2", text: "#4a3a3f", textDim: "#66535a", muted: "#7d6a70", faint: "#8f7c80", accent: "#d9604c", ok: "#4f8f5f", err: "#c93d52", info: "#5b82bd" } },
  { id: "marshmallow-night", label: "Marshmallow Night", mode: "dark", fonts: SERIFY,
    tokens: { bg0: "#231c24", bg1: "#2b2230", bg2: "#342a3a", bg3: "#403447", line: "#4a3d52", lineSoft: "#3a2f42", text: "#f6e9e4", textDim: "#dcc9c8", muted: "#ad9aa6", faint: "#8f7c8c", accent: "#ff9e86", ok: "#8fcf9b", err: "#ff7a8a", info: "#8fb3ee" } },
  { id: "ledger", label: "Ledger", mode: "dark", fonts: SERIFY,
    tokens: { bg0: "#101216", bg1: "#15181d", bg2: "#1a1e24", bg3: "#20252c", line: "#272d36", lineSoft: "#1f242c", text: "#e9e6de", textDim: "#c9c6bd", muted: "#8e939d", faint: "#6e7582", accent: "#d2a24c", ok: "#6fb47c", err: "#e0705f", info: "#7fa8c9" } },
  { id: "blueprint", label: "Blueprint", mode: "dark", fonts: ROUND,
    tokens: { bg0: "#0a0f1a", bg1: "#0e1524", bg2: "#131c30", bg3: "#1a2740", line: "#24314d", lineSoft: "#18223a", text: "#dce6f5", textDim: "#aebfdb", muted: "#7c8fae", faint: "#65789f", accent: "#4fd0e0", ok: "#6fcf97", err: "#e06f7a", info: "#7fa8f0" } },
  { id: "frost", label: "Frost", mode: "dark", fonts: ROUND,
    tokens: { bg0: "#1b2029", bg1: "#232a35", bg2: "#2b3341", bg3: "#353f4f", line: "#3b4557", lineSoft: "#2f3947", text: "#e5ecf4", textDim: "#c2ccd8", muted: "#a2abb7", faint: "#8791a1", accent: "#88c0d0", ok: "#a3be8c", err: "#bf616a", info: "#81a1c1" } },
  { id: "noir-violet", label: "Noir Violet", mode: "dark", fonts: SERIFY,
    tokens: { bg0: "#16131f", bg1: "#1d1929", bg2: "#262035", bg3: "#322a47", line: "#3a3152", lineSoft: "#2a2440", text: "#ece7f5", textDim: "#cbc2e0", muted: "#9b92b2", faint: "#83799d", accent: "#c792ea", ok: "#8fd48a", err: "#e57b8d", info: "#82aaff" } },
  { id: "solarized-dark", label: "Solarized Dark", mode: "dark", fonts: ROUND,
    tokens: { bg0: "#00252e", bg1: "#002b36", bg2: "#073642", bg3: "#0d4a5a", line: "#0f4b5a", lineSoft: "#073642", text: "#eee8d5", textDim: "#93a1a1", muted: "#a7b3b4", faint: "#839ba2", accent: "#b58900", ok: "#859900", err: "#dc322f", info: "#268bd2" } },
  { id: "carbon", label: "Carbon", mode: "dark", fonts: ROUND,
    tokens: { bg0: "#0e0e0f", bg1: "#151517", bg2: "#1c1c1f", bg3: "#252529", line: "#2e2e33", lineSoft: "#202024", text: "#ededf0", textDim: "#c6c6cb", muted: "#8b8b92", faint: "#75757e", accent: "#ffb020", ok: "#57c07a", err: "#e5645a", info: "#6aa9e0" } },
  { id: "midnight-garden", label: "Midnight Garden", mode: "dark", fonts: SERIFY,
    tokens: { bg0: "#0a120c", bg1: "#0f1811", bg2: "#152117", bg3: "#1d2c1f", line: "#28392b", lineSoft: "#19241b", text: "#e9f1e7", textDim: "#c4d4c2", muted: "#8ca38e", faint: "#6f8671", accent: "#f27fa5", ok: "#7cc98a", err: "#e5645a", info: "#7fb8c9" } },
  { id: "porcelain", label: "Porcelain", mode: "light", fonts: ROUND,
    tokens: { bg0: "#eef0f3", bg1: "#f8f9fb", bg2: "#eaedf1", bg3: "#dfe3e9", line: "#d3d8e0", lineSoft: "#e5e8ee", text: "#1e242c", textDim: "#3d4652", muted: "#5e6671", faint: "#6f7b8b", accent: "#3a6ea5", ok: "#3f8a5a", err: "#c0503f", info: "#4f7fb0" } },
  { id: "solarized-light", label: "Solarized Light", mode: "light", fonts: ROUND,
    tokens: { bg0: "#eee8d5", bg1: "#fdf6e3", bg2: "#e7e0cc", bg3: "#dcd4bd", line: "#cfc8ad", lineSoft: "#e0d9c2", text: "#073642", textDim: "#586e75", muted: "#4e5f65", faint: "#647373", accent: "#b58900", ok: "#859900", err: "#dc322f", info: "#268bd2" } },
  { id: "mint-ledger", label: "Mint Ledger", mode: "light", fonts: ROUND,
    tokens: { bg0: "#e6ede8", bg1: "#f2f7f3", bg2: "#dfe9e2", bg3: "#d1ded5", line: "#c3d3c8", lineSoft: "#dbe6df", text: "#1f2a24", textDim: "#3b4a41", muted: "#54635a", faint: "#667a6c", accent: "#2f8f5f", ok: "#3f9a5f", err: "#c0503f", info: "#3d7f96" } },
  { id: "blush-slate", label: "Blush Slate", mode: "light", fonts: SERIFY,
    tokens: { bg0: "#ece7e7", bg1: "#f6f1f1", bg2: "#e6dfdf", bg3: "#dbd2d2", line: "#cec3c3", lineSoft: "#e0d8d8", text: "#2a2426", textDim: "#4a4144", muted: "#62585b", faint: "#7a6b72", accent: "#b05a72", ok: "#5c8a5c", err: "#bf5040", info: "#6a7fa5" } },
  { id: "cozy", label: "Cozy Reading Room", mode: "light", fonts: SERIFY,
    tokens: { bg0: "#ece2d0", bg1: "#f6efe3", bg2: "#fdf6ea", bg3: "#ecdcc7", line: "#e3d6c0", lineSoft: "#ede2cf", text: "#2a1f17", textDim: "#4a3a2c", muted: "#735d4d", faint: "#86755d", accent: "#b8553a", ok: "#5f7d3a", err: "#b5482f", info: "#3d6f96" } },
];

export const DEFAULT_LIGHT = "marshmallow";
export const DEFAULT_DARK = "marshmallow-night";
export const SYSTEM = "system";
export const THEME_PREF_KEY = "hoardini.theme";

const byId = new Map(THEMES.map((t) => [t.id, t]));
export const themeDef = (id: string): ThemeDef => byId.get(id) ?? byId.get(DEFAULT_LIGHT)!;

export function resolveTheme(pref: string, systemDark: boolean): ThemeDef {
  if (pref === SYSTEM) return themeDef(systemDark ? DEFAULT_DARK : DEFAULT_LIGHT);
  return themeDef(pref);
}

export function themeVars(t: ThemeDef): Record<string, string> {
  const k = t.tokens;
  return {
    "--color-ink-950": k.bg0,
    "--color-ink-900": k.bg1,
    "--color-ink-850": k.bg2,
    "--color-ink-800": k.bg3,
    "--color-line": k.line,
    "--color-line-soft": k.lineSoft,
    "--color-paper": k.text,
    "--color-paper-dim": k.textDim,
    "--color-muted": k.muted,
    "--color-faint": k.faint,
    "--color-brass": k.accent,
    "--color-on-accent": k.bg0,
    "--color-ok": k.ok,
    "--color-err": k.err,
    "--color-info": k.info,
    "--font-display": t.fonts[0],
    "--font-sans": t.fonts[1],
    "--font-mono": t.fonts[2],
    "--background": k.bg1,
    "--foreground": k.text,
    "--card": k.bg2,
    "--card-foreground": k.text,
    "--popover": k.bg2,
    "--popover-foreground": k.text,
    "--muted": k.bg3,
    "--muted-foreground": k.muted,
    "--secondary": k.bg3,
    "--secondary-foreground": k.text,
    "--accent": k.bg3,
    "--accent-foreground": k.text,
    "--border": k.line,
    "--input": k.line,
    "--ring": k.accent,
    "--primary": k.accent,
    "--primary-foreground": k.bg0,
    "color-scheme": t.mode,
  };
}

export function applyTheme(t: ThemeDef): void {
  const el = document.documentElement;
  for (const [k, v] of Object.entries(themeVars(t))) el.style.setProperty(k, v);
  el.dataset.theme = t.id;
}

export function themePref(): string {
  try {
    return localStorage.getItem(THEME_PREF_KEY) ?? SYSTEM;
  } catch {
    return SYSTEM;
  }
}

export function saveThemePref(pref: string): void {
  try {
    localStorage.setItem(THEME_PREF_KEY, pref);
  } catch {
    // storage disabled; choice lasts for this session only
  }
}

export const systemPrefersDark = () => window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
