const UNREACHABLE_MESSAGE = "Could not reach the server. It may have stopped; check the app is still running and try again.";
const MAX_BODY_CHARS = 300;

export function isAbortError(err: unknown): boolean {
  return (
    (typeof DOMException !== "undefined" && err instanceof DOMException && err.name === "AbortError") ||
    (err instanceof Error && err.name === "AbortError")
  );
}

// A run that dies before the stream opens rejects instead of emitting RUN_ERROR; lm-ag-ui puts the parsed body on `payload`.
export function describeRunError(err: unknown): string {
  const e = err as { message?: unknown; status?: unknown; payload?: unknown } | null;
  const payload = e?.payload;
  if (payload && typeof payload === "object" && typeof (payload as { error?: unknown }).error === "string") {
    return (payload as { error: string }).error;
  }
  if (typeof payload === "string") {
    const body = payload.trim();
    if (body.length > 0 && body.length <= MAX_BODY_CHARS && !body.startsWith("<")) {
      return typeof e?.status === "number" ? `${body} (HTTP ${e.status})` : body;
    }
  }
  if (err instanceof TypeError) return UNREACHABLE_MESSAGE;
  if (typeof err === "string" && err.trim()) return err;
  if (typeof e?.message === "string" && e.message.trim()) return e.message;
  return "The agent request failed for an unknown reason.";
}

export type TurnEnd = "ok" | "tool-calls" | "no-output";

export interface TurnMessage {
  id: string;
  role: string;
  content?: unknown;
  toolCalls?: unknown[];
}

// "no-output" is the run that completes with nothing at all: no text, no tool calls, no RUN_ERROR.
export function classifySettledTurn(messages: readonly TurnMessage[]): TurnEnd {
  const last = [...messages].reverse().find((m) => m.role !== "tool");
  if (!last) return "ok";
  if (last.role === "user") return "no-output";
  if (last.role !== "assistant") return "ok";
  if (typeof last.content === "string" && last.content.trim()) return "ok";
  return last.toolCalls?.length ? "tool-calls" : "no-output";
}
