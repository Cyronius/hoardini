import { create } from "zustand";

type AiSetup = {
  open: boolean;
  done: boolean | null;
  waitUntil: number;
  wantForm: boolean;
  setOpen(open: boolean): void;
  setDone(done: boolean): void;
  setWait(until: number): void;
  requestForm(): void;
};

export const useAiSetup = create<AiSetup>((set) => ({
  open: false,
  done: null,
  waitUntil: 0,
  wantForm: false,
  setOpen: (open) => set({ open }),
  setDone: (done) => set({ done }),
  setWait: (waitUntil) => set({ waitUntil }),
  requestForm: () => set({ wantForm: true }),
}));
