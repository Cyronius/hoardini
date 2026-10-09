import { GIRTH_MAX, GIRTH_MIN } from "./girth";

export type MascotState = "idle" | "shuffle" | "think" | "tada";

export type Act =
  | MascotState
  | "hop-high"
  | "hop"
  | "hop-heavy"
  | "strain"
  | "shiko"
  | "dizzy"
  | "blink"
  | "sulk"
  | "yawn"
  | "scratch"
  | "stretch"
  | "climb"
  | "push"
  | "sit"
  | "windup"
  | "spread"
  | "fin-push"
  | "fin-flop"
  | "fin-throw"
  | "fin-throw-up"
  | "fin-clap"
  | "slip"
  | "shrug"
  | "pleased"
  | "shake";

export type OneShot = Exclude<Act, MascotState | "climb" | "push" | "sit" | "windup" | "spread">;

export const ACT_MS: Record<OneShot, number> = {
  "hop-high": 820,
  hop: 640,
  "hop-heavy": 900,
  strain: 1600,
  shiko: 1300,
  dizzy: 4400,
  blink: 650,
  sulk: 2400,
  yawn: 2600,
  scratch: 2400,
  stretch: 2600,
  "fin-push": 1800,
  "fin-flop": 1800,
  "fin-throw": 1800,
  "fin-throw-up": 1800,
  "fin-clap": 1800,
  slip: 1900,
  shrug: 1300,
  pleased: 3000,
  shake: 800,
};

// Pre-stretch finisher lengths; Stage scales its parcel motion by FIN_MS / these.
export const FIN_BASE_MS: Record<Move, number> = { push: 1100, flop: 1300, throw: 1300, "throw-up": 1300, clap: 1100 };

export const COMBO_GAP_MS = 1500;
export const FIDGETS = ["yawn", "scratch", "stretch"] as const;

export const girthLevel = (rx: number) => Math.min(1, Math.max(0, (rx - GIRTH_MIN) / (GIRTH_MAX - GIRTH_MIN)));

const MAXED = 0.92;

// Heights are in the sumo's 64-unit viewBox; 13 keeps a skinny jump inside a 56 px header.
export function hopFor(rx: number): { act: "hop-high" | "hop" | "hop-heavy" | "strain"; height: number } {
  const g = girthLevel(rx);
  if (g >= MAXED) return { act: "strain", height: 0 };
  const height = Math.round((13 - (11 * g) / MAXED) * 10) / 10;
  return { act: g < 0.3 ? "hop-high" : g < 0.65 ? "hop" : "hop-heavy", height };
}

export const isFat = (rx: number) => girthLevel(rx) >= 0.65;

export interface Combo {
  count: number;
  at: number;
}

export const nextCombo = (prev: Combo, now: number): Combo => ({ count: now - prev.at < COMBO_GAP_MS ? prev.count + 1 : 1, at: now });

export function pokeAct(count: number, rx: number, playing: OneShot | null, reduced: boolean): OneShot | null {
  if (reduced) {
    if (playing === "sulk") return null;
    return count >= 6 ? "sulk" : "blink";
  }
  if (playing === "dizzy") return null;
  if (count >= 6) return "dizzy";
  if (count === 3) return "shiko";
  return playing ? null : hopFor(rx).act;
}

export type Move = "push" | "flop" | "throw" | "throw-up" | "clap";

export function moveFor(kind: string, destKind?: string | null): Move | null {
  switch (kind) {
    case "delete":
      return "push";
    case "compress":
      return "flop";
    case "archive":
      return destKind === "rclone" ? "throw-up" : "throw";
    case "dedupe":
      return "clap";
    default:
      return null;
  }
}

export const WORK_ACT: Record<Move, Act> = { push: "push", flop: "sit", throw: "windup", "throw-up": "windup", clap: "spread" };
export const FIN_ACT: Record<Move, OneShot> = { push: "fin-push", flop: "fin-flop", throw: "fin-throw", "throw-up": "fin-throw-up", clap: "fin-clap" };

export const reducedMotion = () => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
