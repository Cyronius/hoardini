import { create } from "zustand";
import { ApiError, routes } from "../api/client";
import type { LocationAnswer, Memory, ReviewCardRef, ReviewView } from "../api/types";
import { useStore } from "../store";

type Folder = { path: string; view: ReviewView | null; error: string | null };

interface ReviewState {
  main: ReviewView | null;
  status: "loading" | "ready" | "no-index" | "error";
  error: string | null;
  building: boolean;
  // Cards of a review that a rebuild replaced while the user looked; they stay on screen until reload.
  carry: ReviewCardRef[];
  folder: Folder | null;
  whatIs: string | null;
  open(): Promise<void>;
  refetch(): Promise<void>;
  runNow(): Promise<void>;
  cleanUp(path: string): Promise<void>;
  closeFolder(): void;
  showWhatIs(path: string | null): void;
  answer(path: string, a: LocationAnswer): Promise<Memory>;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export const useReview = create<ReviewState>((set, get) => {
  const accept = (view: ReviewView) => {
    useStore.getState().putProposals(view.proposals);
    const prev = get().main;
    if (prev && prev.review.id !== view.review.id) {
      const ids = new Set(view.cards.map((c) => c.proposal_id));
      const old = prev.review.volume === view.review.volume ? [...get().carry, ...prev.cards] : [];
      const carry = old.filter((c, i) => !ids.has(c.proposal_id) && old.findIndex((o) => o.proposal_id === c.proposal_id) === i);
      set({ carry });
      // The rebuild superseded the untouched ones; the store still holds them as pending.
      for (const c of carry) void useStore.getState().loadProposal(c.proposal_id);
    }
    set({ main: view, status: "ready", error: null });
  };

  return {
    main: null,
    status: "loading",
    error: null,
    building: false,
    carry: [],
    folder: null,
    whatIs: null,

    async open() {
      if (!get().main) set({ status: "loading" });
      try {
        accept(await routes.reviewOpen());
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) set({ main: null, carry: [], status: "no-index", error: null });
        else set({ status: get().main ? "ready" : "error", error: message(e) });
      }
    },
    async refetch() {
      try {
        accept(await routes.review());
      } catch {
        // the next status change or open tries again
      }
    },
    async runNow() {
      set({ building: true, error: null });
      try {
        accept(await routes.reviewRun());
      } catch (e) {
        set({ error: message(e) });
      } finally {
        set({ building: false });
      }
    },
    async cleanUp(path) {
      useStore.getState().setHome("review");
      set({ folder: { path, view: null, error: null } });
      try {
        const view = await routes.reviewRun(path);
        useStore.getState().putProposals(view.proposals);
        if (get().folder?.path === path) set({ folder: { path, view, error: null } });
      } catch (e) {
        if (get().folder?.path === path) set({ folder: { path, view: null, error: message(e) } });
      }
    },
    closeFolder: () => set({ folder: null }),
    showWhatIs(path) {
      if (path) useStore.getState().setHome("review");
      set({ whatIs: path });
    },
    async answer(path, a) {
      const m = await routes.answerLocation(path, a);
      // The server writes the answer into every stored review's areas, so reread the ones on screen.
      const { main, folder } = get();
      await Promise.all([
        main && routes.review(main.review.id).then(accept, () => {}),
        folder?.view &&
          routes.review(folder.view.review.id).then(
            (view) => get().folder?.path === folder.path && set({ folder: { ...folder, view } }),
            () => {},
          ),
      ]);
      return m;
    },
  };
});
