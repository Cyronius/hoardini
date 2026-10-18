import { create } from "zustand";
import { ApiError, routes } from "../api/client";
import type { LocationAnswer, Memory, ReviewCardRef, ReviewView } from "../api/types";
import { selectedVolume } from "../components/mascot/girth";
import { samePath } from "../format";
import { useStore } from "../store";

export type Built = ReviewView & { review: NonNullable<ReviewView["review"]> };

interface ReviewState {
  main: Built | null;
  bare: ReviewView | null;
  status: "loading" | "ready" | "no-index" | "error";
  error: string | null;
  running: boolean;
  building: boolean;
  carry: ReviewCardRef[];
  open(): Promise<void>;
  refetch(): Promise<void>;
  runNow(): Promise<void>;
  changed(): Promise<void>;
  answer(path: string, a: LocationAnswer): Promise<Memory>;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export const useReview = create<ReviewState>((set, get) => {
  const accept = (view: ReviewView) => {
    // Open, refetch and the answer reread race; a reply for another drive or older than what is shown is stale.
    const root = selectedVolume(useStore.getState().status?.volumes)?.root;
    if (root && !samePath(root, view.header.root)) return;
    const prev = get().main;
    const same = !!prev && samePath(prev.header.root, view.header.root);
    if (same && (!view.review || view.review.created_ms < prev.review.created_ms)) return;
    useStore.getState().putProposals(view.proposals);
    const building = !!view.building;
    if (!view.review) {
      set({ main: null, carry: [], bare: view, building, status: "ready", error: null });
      return;
    }
    // Cards that leave the review (replaced, or carried no longer once they ran) stay on screen until reload.
    const ids = new Set(view.cards.map((c) => c.proposal_id));
    const old = same ? [...get().carry, ...prev.cards] : [];
    const carry = old.filter((c, i) => !ids.has(c.proposal_id) && old.findIndex((o) => o.proposal_id === c.proposal_id) === i);
    // A newer review superseded the untouched ones; the store still holds them as pending.
    if (prev && prev.review.id !== view.review.id) for (const c of carry) void useStore.getState().loadProposal(c.proposal_id);
    set({ main: view as Built, bare: null, carry, building, status: "ready", error: null });
  };

  return {
    main: null,
    bare: null,
    status: "loading",
    error: null,
    running: false,
    building: false,
    carry: [],

    async open() {
      if (!get().main && !get().bare) set({ status: "loading" });
      try {
        accept(await routes.reviewOpen());
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) set({ main: null, bare: null, carry: [], building: false, status: "no-index", error: null });
        else set({ status: get().main || get().bare ? "ready" : "error", error: message(e) });
      }
    },
    async refetch() {
      try {
        accept(await routes.review());
      } catch (e) {
        // A first build that ended without a review leaves nothing to show; others retry on the next status change.
        if (e instanceof ApiError && e.status === 404 && !get().main && get().bare)
          set({ bare: null, building: false, status: "error", error: "the build ended without a review" });
      }
    },
    async runNow() {
      set({ running: true, error: null });
      try {
        accept(await routes.reviewRun());
      } catch (e) {
        set({ error: message(e) });
      } finally {
        set({ running: false });
      }
    },
    // Each action already patched the index for the paths it touched, so only the list needs rebuilding.
    async changed() {
      await get().runNow();
    },
    async answer(path, a) {
      const m = await routes.answerLocation(path, a);
      // The server writes the answer into the stored review's areas, so reread the one on screen.
      const { main } = get();
      if (main) await routes.review(main.review.id).then(accept, () => {});
      return m;
    },
  };
});
