import { create } from "zustand";
import type { Message } from "@ag-ui/client";
import { routes } from "./api/client";
import type { IndexSummary, Job, Proposal, Status, StatusEvent, ThreadDetail, ToolData } from "./api/types";
import { saveThemePref, themePref } from "./theme";

export type DrawerTab = "memory" | "destinations" | "providers" | "search" | "theme" | "settings";

export const TADA_MS = 5000;

interface NotedMemory {
  memoryId: number;
  text: string;
  pinned: boolean;
}

export const THREAD_ID = "main";

export type ThreadMeta = Pick<ThreadDetail, "window_start_id" | "has_hidden" | "fresh" | "earlier" | "showing_all">;

// Card payloads ride on stored tool messages (AGENT-TOOL-DATA); live ones arrive as `tool_data` events.
const dataOf = (messages: unknown[]): Record<string, ToolData> => {
  const out: Record<string, ToolData> = {};
  for (const m of messages as { role?: string; toolCallId?: string; data?: ToolData }[]) {
    if (m.role === "tool" && m.toolCallId && m.data) out[m.toolCallId] = m.data;
  }
  return out;
};

const metaOf = (d: ThreadDetail): ThreadMeta => ({
  window_start_id: d.window_start_id,
  has_hidden: d.has_hidden,
  fresh: d.fresh,
  earlier: d.earlier ?? 0,
  showing_all: d.showing_all ?? false,
});

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

  // Bumped when the chat must remount from `seedMessages` (page load, cleared history).
  threadNonce: number;
  seedMessages: Message[] | null;
  threadMeta: ThreadMeta | null;
  chatBusy: boolean;
  runStats: Record<string, { elapsed_ms: number; steps: number }>;
  // The last run ended at the tool-step cap (run_stats.limit_reached), so "keep going" is offered.
  limitHit: boolean;

  proposals: Record<string, Proposal>;
  noted: Record<string, NotedMemory>;
  toolTimes: Record<string, { start: number; end?: number }>;
  toolData: Record<string, ToolData>;
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
  setToolData(toolCallId: string, data: ToolData): void;
  markCallsBefore(messageId: string, callIds: string[]): void;
  loadThread(all?: boolean): Promise<void>;
  refreshThreadMeta(): Promise<void>;
  markAsked(messageId: string, hadMessages: boolean): void;
  clearScreen(): Promise<void>;
  clearHistory(): Promise<void>;
}

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

  threadNonce: 0,
  seedMessages: null,
  threadMeta: null,
  chatBusy: false,
  runStats: {},
  limitHit: false,

  proposals: {},
  noted: {},
  toolTimes: {},
  toolData: {},
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
  setToolData: (toolCallId, data) => set((s) => ({ toolData: { ...s.toolData, [toolCallId]: data } })),
  markCallsBefore: (messageId, callIds) => set((s) => ({ callsBefore: { ...s.callsBefore, [messageId]: callIds } })),
  async loadThread(all = false) {
    try {
      const d = await routes.thread(all);
      set((s) => ({
        threadNonce: s.threadNonce + 1,
        limitHit: false,
        seedMessages: d.messages as Message[],
        threadMeta: metaOf(d),
        proposals: { ...s.proposals, ...d.proposals },
        toolData: { ...s.toolData, ...dataOf(d.messages) },
      }));
    } catch {
      // Offline: still open an empty chat so the composer works once the server is back.
      set((s) => ({ threadNonce: s.threadNonce + 1, seedMessages: null, threadMeta: null }));
    }
  },
  async refreshThreadMeta() {
    try {
      const d = await routes.thread(get().threadMeta?.showing_all ?? false);
      set((s) => ({ threadMeta: metaOf(d), proposals: { ...s.proposals, ...d.proposals } }));
    } catch {
      // keep the last known boundary
    }
  },
  // A question asked while fresh opens the new window; mirror that until the run's refetch.
  markAsked: (messageId, hadMessages) =>
    set((s) =>
      s.threadMeta?.fresh
        ? { threadMeta: { ...s.threadMeta, window_start_id: messageId, has_hidden: s.threadMeta.has_hidden || hadMessages, fresh: false } }
        : s,
    ),
  async clearScreen() {
    await routes.startFresh();
    await get().loadThread(false);
  },
  async clearHistory() {
    await routes.clearHistory();
    await get().loadThread();
  },
}));
