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
  tone?: "midtone";
  tokens: ThemeTokens;
}

const DISPLAY = "'Fraunces Variable', Georgia, serif";
const SANS = "'Nunito Variable', system-ui, sans-serif";
const MONO = "'JetBrains Mono Variable', ui-monospace, Consolas, monospace";
const SERIF = (f: string) => `'${f}', Georgia, serif`;
const SANSF = (f: string) => `'${f}', system-ui, sans-serif`;
const MONOF = (f: string) => `'${f}', ui-monospace, monospace`;
const ROUND: ThemeDef["fonts"] = [SANS, SANS, MONO];
const SERIFY: ThemeDef["fonts"] = [DISPLAY, SANS, MONO];

export const THEMES: ThemeDef[] = [
  { id: "marshmallow", label: "Marshmallow", mode: "light", fonts: SERIFY,
    tokens: { bg0: "#f6ece2", bg1: "#fff8f0", bg2: "#fbeee1", bg3: "#f2dfcf", line: "#e8d3c0", lineSoft: "#f0e1d2", text: "#4a3a3f", textDim: "#66535a", muted: "#7d6a70", faint: "#8f7c80", accent: "#bf3e29", ok: "#4f8f5f", err: "#c93d52", info: "#5b82bd" } },
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
    tokens: { bg0: "#eee8d5", bg1: "#fdf6e3", bg2: "#e7e0cc", bg3: "#dcd4bd", line: "#cfc8ad", lineSoft: "#e0d9c2", text: "#073642", textDim: "#53676e", muted: "#4e5f65", faint: "#647373", accent: "#c29300", ok: "#859900", err: "#dc322f", info: "#268bd2" } },
  { id: "mint-ledger", label: "Mint Ledger", mode: "light", fonts: ROUND,
    tokens: { bg0: "#e6ede8", bg1: "#f2f7f3", bg2: "#dfe9e2", bg3: "#d1ded5", line: "#c3d3c8", lineSoft: "#dbe6df", text: "#1f2a24", textDim: "#3b4a41", muted: "#54635a", faint: "#667a6c", accent: "#35a06a", ok: "#3f9a5f", err: "#c0503f", info: "#3d7f96" } },
  { id: "blush-slate", label: "Blush Slate", mode: "light", fonts: SERIFY,
    tokens: { bg0: "#ece7e7", bg1: "#f6f1f1", bg2: "#e6dfdf", bg3: "#dbd2d2", line: "#cec3c3", lineSoft: "#e0d8d8", text: "#2a2426", textDim: "#4a4144", muted: "#62585b", faint: "#7a6b72", accent: "#a14d64", ok: "#5c8a5c", err: "#bf5040", info: "#6a7fa5" } },
  { id: "cozy", label: "Cozy Reading Room", mode: "light", fonts: SERIFY,
    tokens: { bg0: "#ece2d0", bg1: "#f6efe3", bg2: "#fdf6ea", bg3: "#ecdcc7", line: "#e3d6c0", lineSoft: "#ede2cf", text: "#2a1f17", textDim: "#4a3a2c", muted: "#735d4d", faint: "#86755d", accent: "#a34b33", ok: "#5f7d3a", err: "#b5482f", info: "#3d6f96" } },
  { id: "slate-noon", label: "Slate Noon", mode: "dark", tone: "midtone", fonts: [SANSF("Space Grotesk Variable"), SANSF("IBM Plex Sans Variable"), MONOF("JetBrains Mono Variable")],
    tokens: { bg0: "#363b42", bg1: "#40464e", bg2: "#4b525b", bg3: "#575e68", line: "#626a74", lineSoft: "#4b525b", text: "#f2f4f6", textDim: "#c7ccd2", muted: "#d4d7db", faint: "#b4b9bd", accent: "#5fb0e8", ok: "#7fc98a", err: "#e2776a", info: "#7fb0e0" } },
  { id: "terracotta-dusk", label: "Terracotta Dusk", mode: "dark", tone: "midtone", fonts: [SERIF("Fraunces Variable"), SANSF("IBM Plex Sans Variable"), MONOF("IBM Plex Mono")],
    tokens: { bg0: "#423731", bg1: "#4d4139", bg2: "#594c42", bg3: "#66584c", line: "#6b5b4d", lineSoft: "#524438", text: "#f7ede2", textDim: "#d9c2ac", muted: "#dad1c8", faint: "#c0b2a5", accent: "#e68c5a", ok: "#8fbf6a", err: "#e2705f", info: "#7fa8c9" } },
  { id: "moss-fog", label: "Moss Fog", mode: "dark", tone: "midtone", fonts: [SERIF("Newsreader Variable"), SANSF("Hanken Grotesk Variable"), MONOF("JetBrains Mono Variable")],
    tokens: { bg0: "#363f34", bg1: "#414a3e", bg2: "#4c5648", bg3: "#576352", line: "#576352", lineSoft: "#46503f", text: "#eef2e8", textDim: "#c4cfba", muted: "#d6dbd1", faint: "#b7beaf", accent: "#7bc25c", ok: "#7bc25c", err: "#d97a63", info: "#7fa8c9" } },
  { id: "graphite-ember", label: "Graphite Ember", mode: "dark", tone: "midtone", fonts: [SERIF("Spectral"), SANSF("IBM Plex Sans Variable"), MONOF("IBM Plex Mono")],
    tokens: { bg0: "#332d28", bg1: "#3d3630", bg2: "#47403a", bg3: "#524a42", line: "#5c5045", lineSoft: "#453c34", text: "#f5efe6", textDim: "#d3c3ae", muted: "#c6b9ac", faint: "#b09e8c", accent: "#e8a33d", ok: "#8fbf6a", err: "#d97361", info: "#7fa0c4" } },
  { id: "concrete-violet", label: "Concrete Violet", mode: "dark", tone: "midtone", fonts: [SANSF("Space Grotesk Variable"), SANSF("Hanken Grotesk Variable"), MONOF("JetBrains Mono Variable")],
    tokens: { bg0: "#3a3742", bg1: "#45414d", bg2: "#504c59", bg3: "#5c5765", line: "#665f74", lineSoft: "#4d495a", text: "#f1eff6", textDim: "#cbc6d6", muted: "#d2cfd9", faint: "#b4b1be", accent: "#b591e4", ok: "#7fc98a", err: "#e2776a", info: "#8ea3f0" } },
  { id: "sandstone-teal", label: "Sandstone Teal", mode: "dark", tone: "midtone", fonts: [SANSF("Hanken Grotesk Variable"), SANSF("Hanken Grotesk Variable"), MONOF("JetBrains Mono Variable")],
    tokens: { bg0: "#423b2e", bg1: "#4d4536", bg2: "#584f3f", bg3: "#635948", line: "#6b5e48", lineSoft: "#524836", text: "#f6efe1", textDim: "#d6c7a9", muted: "#d9d1c4", faint: "#bfb3a0", accent: "#37b8ab", ok: "#8fbf6a", err: "#d9704f", info: "#6fa8c0" } },
  { id: "blue-hour", label: "Blue Hour", mode: "dark", tone: "midtone", fonts: [SERIF("Fraunces Variable"), SANSF("IBM Plex Sans Variable"), MONOF("JetBrains Mono Variable")],
    tokens: { bg0: "#2f3648", bg1: "#394153", bg2: "#434c5f", bg3: "#4e576b", line: "#576285", lineSoft: "#414b68", text: "#eef1fa", textDim: "#c3cbe0", muted: "#c7ccdb", faint: "#a9afc4", accent: "#8ea3f0", ok: "#7fc9a0", err: "#e07f7f", info: "#8ea3f0" } },
  { id: "clay-ash", label: "Clay Ash", mode: "dark", tone: "midtone", fonts: [SERIF("Newsreader Variable"), SANSF("IBM Plex Sans Variable"), MONOF("IBM Plex Mono")],
    tokens: { bg0: "#3e3230", bg1: "#493c39", bg2: "#544642", bg3: "#60504b", line: "#6b5651", lineSoft: "#52423e", text: "#f5e9e5", textDim: "#d6bcb5", muted: "#d2c4c1", faint: "#bba7a3", accent: "#81a771", ok: "#7ba36b", err: "#d9705a", info: "#7fa0c4" } },
  { id: "rosewood", label: "Rosewood", mode: "dark", tone: "midtone", fonts: [SERIF("Fraunces Variable"), SANSF("IBM Plex Sans Variable"), MONOF("IBM Plex Mono")],
    tokens: { bg0: "#3d3034", bg1: "#483a3e", bg2: "#544349", bg3: "#604c53", line: "#6b535a", lineSoft: "#4f4046", text: "#f5e9ec", textDim: "#d6bcc2", muted: "#cec0c3", faint: "#b7a4a8", accent: "#b54361", ok: "#8fbf6a", err: "#d9705f", info: "#7fa0c4" } },
  { id: "coastal-slate", label: "Coastal Slate", mode: "dark", tone: "midtone", fonts: [SERIF("Zilla Slab"), SANSF("IBM Plex Sans Variable"), MONOF("IBM Plex Mono")],
    tokens: { bg0: "#2e3a3c", bg1: "#384547", bg2: "#425052", bg3: "#4d5c5e", line: "#576769", lineSoft: "#414f51", text: "#e9f2f2", textDim: "#c0d0d0", muted: "#c8d2d2", faint: "#a7b6b7", accent: "#4fb0a0", ok: "#4fb0a0", err: "#e2776a", info: "#7fa0c4" } },
  { id: "radium-bloom", label: "Radium Bloom", mode: "dark", tone: "midtone", fonts: [SANSF("Audiowide"), SANSF("Outfit Variable"), MONOF("JetBrains Mono Variable")],
    tokens: { bg0: "#3d1046", bg1: "#4f1659", bg2: "#631d6e", bg3: "#7a2686", line: "#9236a0", lineSoft: "#6e2079", text: "#fdf0ff", textDim: "#e3b8f0", muted: "#d3b0dd", faint: "#b597bf", accent: "#ccff00", ok: "#00e6a0", err: "#ff2d6b", info: "#00e5ff" } },
  { id: "thundercloud", label: "Thundercloud", mode: "dark", tone: "midtone", fonts: [SANSF("Oswald Variable"), SANSF("Source Sans 3 Variable"), MONOF("Cousine")],
    tokens: { bg0: "#333a45", bg1: "#3e4653", bg2: "#49515f", bg3: "#555e6d", line: "#646e80", lineSoft: "#49515e", text: "#f1f5f9", textDim: "#ced6df", muted: "#ced4dc", faint: "#abb3bf", accent: "#ffd23f", ok: "#7fc98a", err: "#e57373", info: "#7fb3e8" } },
  { id: "ink-wash", label: "Ink Wash", mode: "dark", tone: "midtone", fonts: [SERIF("Noto Serif JP Variable"), SANSF("Zen Maru Gothic"), MONOF("M PLUS 1 Code Variable")],
    tokens: { bg0: "#3b3b38", bg1: "#454542", bg2: "#504f4c", bg3: "#5c5b57", line: "#6a6965", lineSoft: "#4b4a47", text: "#f4f2ee", textDim: "#d9d4cd", muted: "#d0ccc5", faint: "#b1ada6", accent: "#d8220d", ok: "#7fb573", err: "#c22e20", info: "#6f9fb5" } },
  { id: "lava-lamp", label: "Lava Lamp", mode: "dark", tone: "midtone", fonts: [SERIF("Caprasimo"), SANSF("Baloo 2 Variable"), MONOF("Azeret Mono Variable")],
    tokens: { bg0: "#46344e", bg1: "#523d5b", bg2: "#5e4768", bg3: "#6b5276", line: "#7b6087", lineSoft: "#584261", text: "#f7eef9", textDim: "#e1cde8", muted: "#dccce2", faint: "#bea8c6", accent: "#ff8c3a", ok: "#a8d94f", err: "#ff5a6b", info: "#6fb8e8" } },
  { id: "marrakech", label: "Marrakech", mode: "dark", tone: "midtone", fonts: [SERIF("Amiri"), SANSF("Rubik Variable"), MONOF("Cousine")],
    tokens: { bg0: "#5c4326", bg1: "#694e2e", bg2: "#775a37", bg3: "#856640", line: "#957447", lineSoft: "#6d5231", text: "#fbf1df", textDim: "#edd7b1", muted: "#e7d7ba", faint: "#cdb893", accent: "#385cff", ok: "#8fb573", err: "#d9503f", info: "#3fbfb0" } },
  { id: "canyon", label: "Canyon", mode: "dark", tone: "midtone", fonts: [SERIF("Eczar Variable"), SANSF("Rubik Variable"), MONOF("PT Mono")],
    tokens: { bg0: "#5a3d31", bg1: "#66473a", bg2: "#735244", bg3: "#7f5d4e", line: "#8e6b5a", lineSoft: "#6a4b3d", text: "#faeee5", textDim: "#e6cdb9", muted: "#e3d3c6", faint: "#c9b3a4", accent: "#63bae9", ok: "#93bf6a", err: "#e05a45", info: "#8fc9e8" } },
  { id: "periwinkle-dusk", label: "Periwinkle Dusk", mode: "dark", tone: "midtone", fonts: [SERIF("Lora Variable"), SANSF("Mulish Variable"), MONOF("Azeret Mono Variable")],
    tokens: { bg0: "#454a63", bg1: "#50556f", bg2: "#5b617f", bg3: "#676d8e", line: "#777da0", lineSoft: "#565c78", text: "#f2f3fa", textDim: "#dbdeee", muted: "#d5d8e8", faint: "#b5b9d1", accent: "#ffb08a", ok: "#8fd49f", err: "#e87a7a", info: "#9fb3f2" } },
  { id: "ultramarine", label: "Ultramarine", mode: "dark", tone: "midtone", fonts: [SERIF("Playfair Display Variable"), SANSF("Jost Variable"), MONOF("Fragment Mono")],
    tokens: { bg0: "#2b3168", bg1: "#333a78", bg2: "#3c4489", bg3: "#464f9b", line: "#5b66b4", lineSoft: "#39408a", text: "#f0eff8", textDim: "#c6c8ea", muted: "#cfd1ee", faint: "#adb0d6", accent: "#ff9a3c", ok: "#64dba0", err: "#ff6b8a", info: "#7fd0ff" } },
  { id: "cinnabar", label: "Cinnabar", mode: "dark", tone: "midtone", fonts: [SERIF("Noto Serif JP Variable"), SANSF("Rubik Variable"), MONOF("M PLUS 1 Code Variable")],
    tokens: { bg0: "#4d2823", bg1: "#5a2f29", bg2: "#683730", bg3: "#764038", line: "#8b4e44", lineSoft: "#613229", text: "#fdeee2", textDim: "#edc9ae", muted: "#ecd2bf", faint: "#d0b09c", accent: "#f0c14b", ok: "#86c98f", err: "#ff7a63", info: "#7fc0d9" } },
  { id: "cyberdeck", label: "Cyberdeck", mode: "dark", tone: "midtone", fonts: [SANSF("Unbounded Variable"), SANSF("Outfit Variable"), MONOF("Azeret Mono Variable")],
    tokens: { bg0: "#2f3740", bg1: "#38414b", bg2: "#424c58", bg3: "#4d5865", line: "#5f6d7d", lineSoft: "#3e4854", text: "#eff5fa", textDim: "#c3d0dc", muted: "#ccd8e2", faint: "#a8b6c4", accent: "#c6f24a", ok: "#5fe0a0", err: "#ff6f9a", info: "#59d8ff" } },
  { id: "kraft", label: "Kraft", mode: "light", tone: "midtone", fonts: [SERIF("Zilla Slab"), SANSF("Instrument Sans Variable"), MONOF("Fragment Mono")],
    tokens: { bg0: "#ae9971", bg1: "#bfa87f", bg2: "#b39c72", bg3: "#a68f66", line: "#86714b", lineSoft: "#b7a077", text: "#241c10", textDim: "#3b2f1b", muted: "#4c3e24", faint: "#665538", accent: "#5e1d0d", ok: "#35631f", err: "#a03018", info: "#1f4f7a" } },
  { id: "sage-chalk", label: "Sage Chalk", mode: "light", tone: "midtone", fonts: [SERIF("Newsreader Variable"), SANSF("Karla Variable"), MONOF("Sometype Mono Variable")],
    tokens: { bg0: "#94a890", bg1: "#a3b79f", bg2: "#99ad95", bg3: "#8da189", line: "#74896f", lineSoft: "#9db19a", text: "#131e15", textDim: "#26332a", muted: "#37473a", faint: "#4e6250", accent: "#5e2641", ok: "#24632f", err: "#96301f", info: "#23557a" } },
  { id: "tater", label: "Tater Dog", mode: "light", fonts: [SANSF("Quicksand Variable"), SANSF("Quicksand Variable"), MONOF("JetBrains Mono Variable")],
    tokens: { bg0: "#bce0c8", bg1: "#ffffff", bg2: "#d6ecdd", bg3: "#e0c89a", line: "#2a2620", lineSoft: "#cfe0d4", text: "#1a1714", textDim: "#3f342a", muted: "#594531", faint: "#756854", accent: "#7a4424", ok: "#3f7d4a", err: "#c0432b", info: "#3a6b8f" } },
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

export type ThemeGroup = "dark" | "midtone" | "light";

// Tone is checked before mode: Kraft and Sage Chalk are mode "light" but sit in the midtone group.
export const groupOf = (t: ThemeDef): ThemeGroup => (t.tone === "midtone" ? "midtone" : t.mode);

function luminance(hex: string): number {
  const lin = (i: number) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(1) + 0.7152 * lin(3) + 0.0722 * lin(5);
}

export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

export function onAccent(k: ThemeTokens): string {
  return contrast(k.bg0, k.accent) >= contrast(k.text, k.accent) ? k.bg0 : k.text;
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
    "--color-on-accent": onAccent(k),
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
    "--primary-foreground": onAccent(k),
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
