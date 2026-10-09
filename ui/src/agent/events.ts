import type { RequestHandler } from "@itkennel/lm-ag-ui";
import { readSse } from "../api/client";
import { useAiSetup } from "../aiSetup";
import { useStore } from "../store";

let runId: string | null = null;
export const currentRunId = () => runId;
let runCalls: string[] = [];

function handle(data: string): void {
  let ev: { type?: string; [k: string]: unknown };
  try {
    ev = JSON.parse(data);
  } catch {
    return;
  }
  const s = useStore.getState();
  switch (ev.type) {
    case "RUN_STARTED":
      runId = String(ev.runId ?? "") || null;
      runCalls = [];
      useStore.setState({ limitHit: false });
      break;
    case "RUN_FINISHED":
    case "RUN_ERROR":
      useAiSetup.getState().setWait(0);
      break;
    case "TEXT_MESSAGE_START":
      useAiSetup.getState().setWait(0);
      if (runCalls.length) s.markCallsBefore(String(ev.messageId), runCalls);
      break;
    case "TOOL_CALL_START":
      runCalls = [...runCalls, String(ev.toolCallId)];
      s.markToolStart(String(ev.toolCallId));
      break;
    case "TOOL_CALL_RESULT":
      s.markToolEnd(String(ev.toolCallId));
      break;
    case "CUSTOM": {
      const v = (ev.value ?? {}) as Record<string, unknown>;
      if (ev.name === "proposal" && typeof v.proposal_id === "string") void s.loadProposal(v.proposal_id);
      else if (ev.name === "rate_limit_wait") useAiSetup.getState().setWait(Date.now() + Number(v.seconds) * 1000);
      else if (ev.name === "memory_noted" && typeof v.tool_call_id === "string") {
        s.noteMemory(v.tool_call_id, { memoryId: Number(v.memory_id), text: String(v.text ?? "") });
      } else if (ev.name === "run_stats" && runId) {
        useStore.setState((st) => ({
          runStats: { ...st.runStats, [runId!]: { elapsed_ms: Number(v.elapsed_ms), steps: Number(v.steps) } },
          limitHit: v.limit_reached === true,
        }));
      }
      break;
    }
  }
}

// lm-ag-ui does not surface CUSTOM events, so we read a tee of the same SSE body.
export const requestHandler: RequestHandler = async (url, init) => {
  const res = await fetch(url, init);
  if (!res.body || !(res.headers.get("content-type") ?? "").includes("text/event-stream")) return res;
  const [forClient, forUs] = res.body.tee();
  void readSse(forUs, handle).catch(() => {});
  return new Response(forClient, { status: res.status, statusText: res.statusText, headers: res.headers });
};
