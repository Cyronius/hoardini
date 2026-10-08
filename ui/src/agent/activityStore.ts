import { create } from "zustand";

export interface LiveStep {
  id: string;
  kind: "thinking" | "tool";
  label: string;
}

type LifecycleEvent =
  | { type: "run_started" }
  | { type: "tool_used"; toolName: string }
  | { type: "message_added"; role: string; content: string };

interface ActivityState {
  steps: LiveStep[];
  onLifecycle(event: LifecycleEvent): void;
  clear(): void;
}

let seq = 0;
const nextId = () => `act_${seq++}`;

export const useActivity = create<ActivityState>((set) => ({
  steps: [],
  onLifecycle(event) {
    switch (event.type) {
      case "run_started":
        // Reset per run: the prior run's calls are already committed to `messages`.
        set({ steps: [{ id: nextId(), kind: "thinking", label: "Thinking" }] });
        break;
      case "tool_used":
        set((s) => ({ steps: [...s.steps, { id: nextId(), kind: "tool", label: event.toolName }] }));
        break;
      case "message_added":
        set((s) => ({ steps: s.steps.filter((st, i) => !(st.kind === "thinking" && i === s.steps.length - 1)) }));
        break;
    }
  },
  clear() {
    set({ steps: [] });
  },
}));
