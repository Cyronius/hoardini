import { create } from "zustand";
import type { Message } from "@ag-ui/client";
import { routes } from "./api/client";
import type { IndexSummary, Job, Proposal, Status, StatusEvent } from "./api/types";
import { saveThemePref, themePref } from "./theme";

export type DrawerTab = "memory" | "destinations" | "providers" | "search" | "theme" | "settings";

export const TADA_MS = 5000;

interface NotedMemory {
  memoryId: number;
  text: string;
  pinned: boolean;
}

interface State {
  status: Status | null;
  connected: boolean;
  summary: IndexSummary | null;
  jobs: Record<string, Job>;
  tadaUntil: number;
  mascotAway: boolean;
  setMascotAway(away: boolean): void;

  drawer: DrawerTab | null;
  themePref: string;

  threadId: string;
  threadNonce: number;
  seedMessages: Message[] | null;
  chatBusy: boolean;
  runStats: Record<string, { elapsed_ms: number; steps: number }>;
  // The last run ended at the tool-step cap (run_stats.limit_reached), so "keep going" is offered.
  limitHit: boolean;

  proposals: Record<string, Proposal>;
  noted: Record<string, NotedMemory>;
  toolTimes: Record<string, { start: number; end?: number }>;
  // lm-ag-ui folds a text-less tool round into the next text message; these calls came before that text.
  callsBefore: Record<string, string[]>;

  applyStatusEvent(e: StatusEvent): void;
  setConnected(c: boolean): void;
  setSummary(s: IndexSummary | null): void;
  openDrawer(tab: DrawerTab | null): void;
  setThemePref(p: string): void;
  setChatBusy(b: boolean): void;
  putProposal(p: Proposal): void;
  loadProposal(id: string): Promise<Proposal | null>;
  noteMemory(toolCallId: string, n: NotedMemory): void;
  markToolStart(id: string): void;
  markToolEnd(id: string): void;
  markCallsBefore(messageId: string, callIds: string[]): void;
  newThread(): void;
  openThread(id: string): Promise<void>;
}

export const newThreadId = () => `thread-${crypto.randomUUID()}`;

export const useStore = create<State>((set, get) => ({
  status: null,
  connected: false,
  summary: null,
  jobs: {},
  tadaUntil: 0,
  mascotAway: false,
  setMascotAway: (mascotAway) => set({ mascotAway }),

  drawer: null,
  themePref: themePref(),

  threadId: newThreadId(),
  threadNonce: 0,
  seedMessages: null,
  chatBusy: false,
  runStats: {},
  limitHit: false,

  proposals: {},
  noted: {},
  toolTimes: {},
  callsBefore: {},

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
  putProposal: (p) => set((s) => ({ proposals: { ...s.proposals, [p.id]: p } })),
  async loadProposal(id) {
    try {
      const p = await routes.proposal(id);
      get().putProposal(p);
      return p;
    } catch {
      return null;
    }
  },
  noteMemory: (toolCallId, n) => set((s) => ({ noted: { ...s.noted, [toolCallId]: n } })),
  markToolStart: (id) => set((s) => ({ toolTimes: { ...s.toolTimes, [id]: { start: Date.now() } } })),
  markToolEnd: (id) =>
    set((s) => {
      const t = s.toolTimes[id];
      return t && !t.end ? { toolTimes: { ...s.toolTimes, [id]: { ...t, end: Date.now() } } } : s;
    }),
  markCallsBefore: (messageId, callIds) => set((s) => ({ callsBefore: { ...s.callsBefore, [messageId]: callIds } })),
  newThread: () => set((s) => ({ threadId: newThreadId(), threadNonce: s.threadNonce + 1, seedMessages: null, limitHit: false })),
  async openThread(id) {
    const detail = await routes.thread(id);
    set((s) => ({
      threadId: detail.id,
      threadNonce: s.threadNonce + 1,
      limitHit: false,
      seedMessages: detail.messages as Message[],
      proposals: { ...s.proposals, ...detail.proposals },
    }));
  },
}));
