export const TEXT_SIZES = [
  { id: "small", label: "Small" },
  { id: "medium", label: "Medium" },
  { id: "large", label: "Large" },
] as const;

export type TextSize = (typeof TEXT_SIZES)[number]["id"];

const KEY = "hoardini.textSize";

export function textSizePref(): TextSize {
  try {
    const v = localStorage.getItem(KEY);
    return TEXT_SIZES.some((s) => s.id === v) ? (v as TextSize) : "medium";
  } catch {
    return "medium";
  }
}

export function applyTextSize(s: TextSize): void {
  document.documentElement.dataset.textSize = s;
}

export function saveTextSize(s: TextSize): void {
  applyTextSize(s);
  try {
    localStorage.setItem(KEY, s);
  } catch {
    // storage disabled; choice lasts for this session only
  }
}
