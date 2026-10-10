export const WIDE_MIN_PX = 1100;
export const CHAT_DEFAULT_PX = 420;
export const CHAT_MIN_PX = 340;
export const CHAT_MAX_PX = 760;
export const HOME_MIN_PX = 560;

export type LayoutPrefs = { chatWidth: number | null; collapsed: boolean };

const KEY = "hoardini.layout";
const DEFAULTS: LayoutPrefs = { chatWidth: null, collapsed: false };

export const isWide = (windowWidth: number) => windowWidth >= WIDE_MIN_PX;

// The home pane keeps HOME_MIN_PX, so a narrow wide window squeezes the panel before the review.
export function chatWidthFor(windowWidth: number, want: number | null): number {
  const max = Math.max(CHAT_MIN_PX, Math.min(CHAT_MAX_PX, windowWidth - HOME_MIN_PX));
  return Math.round(Math.min(max, Math.max(CHAT_MIN_PX, want ?? CHAT_DEFAULT_PX)));
}

export function parsePrefs(raw: string | null): LayoutPrefs {
  try {
    const v = JSON.parse(raw ?? "null") as Partial<Record<keyof LayoutPrefs, unknown>> | null;
    if (!v || typeof v !== "object") return DEFAULTS;
    const w = v.chatWidth;
    return { chatWidth: typeof w === "number" && Number.isFinite(w) && w > 0 ? w : null, collapsed: v.collapsed === true };
  } catch {
    return DEFAULTS;
  }
}

export function loadPrefs(): LayoutPrefs {
  try {
    return parsePrefs(localStorage.getItem(KEY));
  } catch {
    return DEFAULTS;
  }
}

export function savePrefs(p: LayoutPrefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    // storage disabled; the layout lasts for this page load
  }
}
