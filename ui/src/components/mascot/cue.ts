import { create } from "zustand";
import { ACT_MS, type OneShot } from "./acts";
import type { Volume } from "./girth";

interface Cue {
  act: OneShot;
  n: number;
}

interface MascotStage {
  cue: Cue | null;
  headerWork: boolean;
  // Girth stays at the pre-job size until the finisher, so space freed mid-job deflates in one go.
  hold: Volume | null;
  // He is already pleased with himself, so the rest of this tada is skipped.
  quietTadaUntil: number;
}

export const useMascotStage = create<MascotStage>(() => ({ cue: null, headerWork: false, hold: null, quietTadaUntil: 0 }));

let seq = 0;
let timer: ReturnType<typeof setTimeout> | undefined;

export function cueHeader(act: OneShot, ms: number = ACT_MS[act]): void {
  const cue = { act, n: ++seq };
  clearTimeout(timer);
  useMascotStage.setState({ cue });
  timer = setTimeout(() => {
    if (useMascotStage.getState().cue?.n === cue.n) useMascotStage.setState({ cue: null });
  }, ms);
}

export const holdGirth = (v: Volume | undefined) => {
  if (v && !useMascotStage.getState().hold) useMascotStage.setState({ hold: v });
};
export const releaseGirth = () => useMascotStage.setState({ hold: null });
