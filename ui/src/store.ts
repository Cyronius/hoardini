import { create } from "zustand";
import { routes } from "./api/client";
import type { IndexSummary, Job, Proposal, Status, StatusEvent } from "./api/types";
import { saveThemePref, themePref } from "./theme";

export type DrawerTab = "main" | "memory" | "destinations" | "providers" | "search" | "settings";

export const TADA_MS = 5000;

// A review refetch can land between a tick and its PUT's reply; the tick on screen must not flip back.
const held = new Map<string, Set<number>>();
export const heldTicks = (id: string) => held.get(id) ?? null;
export function holdTicks(id: string, ids: Set<number> | null) {
  if (ids) held.set(id, ids);
  else held.delete(id);
}
const withHeld = (p: Proposal): Proposal => {
  const h = held.get(p.id);
  return h && p.status === "pending" ? { ...p, items: p.items.map((i) => (i.status === "pending" ? { ...i, selected: h.has(i.id) } : i)) } : p;
};

interface State {
  status: Status | null;
  connected: boolean;
  summary: IndexSummary | null;
  jobs: Record<string, Job>;
  tadaUntil: number;
  mascotAway: boolean;
  approving: string | null;
  // A hold-to-confirm button is being held; Hoardini carries the box meanwhile.
  holding: boolean;
  setHolding(h: boolean): void;
  // Home has nothing to show, so Hoardini naps.
  homeClean: boolean;
  setMascotAway(away: boolean): void;
  setApproving(proposalId: string | null): void;

  drawer: DrawerTab | null;
  themePref: string;
  // An item question is being answered (AGENT-ASK); Hoardini stomps while he thinks.
  chatBusy: boolean;

  proposals: Record<string, Proposal>;

  applyStatusEvent(e: StatusEvent): void;
  setConnected(c: boolean): void;
  setSummary(s: IndexSummary | null): void;
  openDrawer(tab: DrawerTab | null): void;
  setThemePref(p: string): void;
  setChatBusy(b: boolean): void;
  putProposal(p: Proposal): void;
  putProposals(ps: Record<string, Proposal>): void;
  loadProposal(id: string): Promise<Proposal | null>;
}

export const useStore = create<State>((set, get) => ({
  status: null,
  connected: false,
  summary: null,
  jobs: {},
  tadaUntil: 0,
  mascotAway: false,
  approving: null,
  holding: false,
  setHolding: (holding) => set({ holding }),
  homeClean: false,
  setApproving: (approving) => set({ approving }),
  setMascotAway: (mascotAway) => set({ mascotAway }),

  drawer: null,
  themePref: themePref(),
  chatBusy: false,

  proposals: {},

  applyStatusEvent(e) {
    if (e.type === "status") {
      const { type: _t, ...status } = e;
      set((s) => {
        const jobs = { ...s.jobs };
        for (const j of status.jobs) jobs[j.id] = j;
        return { status, jobs };
      });
      return;
    }
    const { type: _t, ...job } = e;
    const prev = get().jobs[job.id];
    const finishedNow = job.status === "done" && prev?.status !== "done" && job.freed_bytes > 0;
    set((s) => ({
      jobs: { ...s.jobs, [job.id]: job },
      status: s.status ? { ...s.status, jobs: [job, ...s.status.jobs.filter((j) => j.id !== job.id)] } : s.status,
      tadaUntil: finishedNow ? Date.now() + TADA_MS : s.tadaUntil,
    }));
    if (job.proposal_id && ["done", "failed", "cancelled"].includes(job.status)) {
      void get().loadProposal(job.proposal_id);
    }
  },
  setConnected: (connected) => set({ connected }),
  setSummary: (summary) => set({ summary }),
  openDrawer: (drawer) => set({ drawer }),
  setThemePref(p) {
    saveThemePref(p);
    set({ themePref: p });
  },
  setChatBusy: (chatBusy) => set({ chatBusy }),
  putProposal: (p) => set((s) => ({ proposals: { ...s.proposals, [p.id]: withHeld(p) } })),
  putProposals: (ps) => set((s) => ({ proposals: { ...s.proposals, ...Object.fromEntries(Object.entries(ps).map(([k, p]) => [k, withHeld(p)])) } })),
  async loadProposal(id) {
    try {
      const p = await routes.proposal(id);
      get().putProposal(p);
      return p;
    } catch {
      return null;
    }
  },
}));
