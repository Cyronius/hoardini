import { create } from "zustand";
import { ApiError, routes } from "../api/client";
import type { LocationAnswer, Memory, ReviewCardRef, ReviewView } from "../api/types";
import { selectedVolume } from "../components/mascot/girth";
import { samePath } from "../format";
import { useStore } from "../store";

export type Built = ReviewView & { review: NonNullable<ReviewView["review"]> };
type Folder = { path: string; view: Built | null; error: string | null; running: boolean };

interface ReviewState {
  main: Built | null;
  bare: ReviewView | null;
  status: "loading" | "ready" | "no-index" | "error";
  error: string | null;
  running: boolean;
  building: boolean;
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
  markAsked(): void;
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
    folder: null,
    whatIs: null,

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
    async cleanUp(path) {
      useStore.getState().setHome("review");
      const prev = get().folder;
      const shown = prev && samePath(prev.path, path) ? prev.view : null;
      if (prev?.running && shown) return;
      set({ folder: { path, view: shown, error: null, running: true } });
      try {
        const view = (await routes.reviewRun(path)) as Built;
        useStore.getState().putProposals(view.proposals);
        if (get().folder?.path === path) set({ folder: { path, view, error: null, running: false } });
      } catch (e) {
        if (get().folder?.path === path) set({ folder: { path, view: shown, error: message(e), running: false } });
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
            (view) => get().folder?.path === folder.path && set({ folder: { ...folder, view: view as Built } }),
            () => {},
          ),
      ]);
      return m;
    },
    // The views on screen keep the `asked` they were fetched with.
    markAsked() {
      const asked = <V extends ReviewView>(v: V | null): V | null => (v?.online && !v.online.asked ? { ...v, online: { ...v.online, asked: true } } : v);
      const { main, bare, folder } = get();
      set({ main: asked(main), bare: asked(bare), folder: folder && { ...folder, view: asked(folder.view) } });
    },
  };
});
