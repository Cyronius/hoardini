import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { AgentProvider, useAgent, useAgentContext } from "@itkennel/lm-ag-ui";
import type { Message } from "@ag-ui/client";
import { getToken, routes } from "../api/client";
import type { ProviderView } from "../api/types";
import { resolveMaxSteps, resolveStallMs, WATCHDOG_BACKSTOP_MS } from "../agent/aiTimeouts";
import { useActivity } from "../agent/activityStore";
import { setRestoredSink } from "../agent/decisions";
import { currentRunId, requestHandler, rerouteNextRun } from "../agent/events";
import { proposalTools } from "../agent/proposalTools";
import { classifySettledTurn, describeRunError, isAbortError } from "../agent/runError";
import { isProposeTool } from "../agent/toolDefs";
import { friendlyToolName, humanElapsed } from "../format";
import { useExploreLink } from "../explore/context";
import { THREAD_ID, useStore } from "../store";
import { FollowUps, followUps } from "./FollowUps";
import { Starters } from "./Greeting";
import { EraserIcon, SendIcon, SpinnerIcon, StopIcon } from "./icons";
import { IconButton } from "./IconButton";
import { LearnedChip } from "./LearnedChip";
import { Markdown } from "./Markdown";
import { ToolCard, isCardData } from "./ToolCards";
import { ToolChip } from "./ToolChip";

const sentToolIds = new Set<string>();

// Server tool results ("tool_result_*") and restored history are already on the server; only frontend-tool answers go out.
function pruneOutbound(messages: Message[]): Message[] {
  return messages.filter((m) => {
    if (m.role !== "tool") return true;
    if (!m.id.startsWith("tool_") || m.id.startsWith("tool_result_") || sentToolIds.has(m.id)) return false;
    sentToolIds.add(m.id);
    return true;
  });
}

type ToolCallLike = { id: string; function?: { name?: string; arguments?: string } };

function LiveActivity() {
  const steps = useActivity((s) => s.steps);
  const { currentMessage } = useAgentContext();
  const visible = currentMessage && steps.length && steps[steps.length - 1]!.kind === "thinking" ? steps.slice(0, -1) : steps;
  if (visible.length === 0) {
    if (currentMessage) return null;
    return (
      <div className="fade-up flex items-center gap-2 text-[length:var(--fs-sm)]">
        <SpinnerIcon />
        <span className="font-semibold text-muted pulse-soft">Working...</span>
      </div>
    );
  }
  return (
    <div className="fade-up space-y-1">
      {visible.map((st, i) => {
        const active = i === visible.length - 1;
        return (
          <div key={st.id} className="flex items-center gap-2 text-[length:var(--fs-sm)]">
            {active ? <SpinnerIcon /> : <span className="shrink-0 text-ok">✓</span>}
            <span className="font-semibold text-muted">{st.kind === "tool" ? friendlyToolName(st.label) : st.label}</span>
            {active && <span className="text-faint pulse-soft">...</span>}
          </div>
        );
      })}
    </div>
  );
}

function Prompt({ children, actions }: { children: ReactNode; actions: ReactNode }) {
  return (
    <div className="fade-up rounded-xl border border-brass/30 bg-brass/10 px-3 py-2 text-[length:var(--fs-sm)]">
      <div className="text-paper-dim">{children}</div>
      <div className="mt-2 flex gap-2">{actions}</div>
    </div>
  );
}

function Boundary() {
  return (
    <div role="separator" className="flex items-center gap-3 py-1 text-[length:var(--fs-xs)] font-semibold text-faint">
      <span className="h-px flex-1 bg-line" />
      <span>Hoardini doesn't remember above this line</span>
      <span className="h-px flex-1 bg-line" />
    </div>
  );
}

const btnAccent ="rounded-lg border border-brass/40 px-2.5 py-1 font-bold text-brass hover:bg-brass/10";
const btnQuiet = "rounded-lg border border-line px-2.5 py-1 font-semibold text-muted hover:text-paper";

function Chat({ limits }: { limits: { stallMs: number; maxSteps: number } }) {
  const { messages, currentMessage, isStreaming, addMessage, agentClient, agentSubscriber, tools: toolDefs, getForwardedProps, getToolNameFromCallId, terminateRun } =
    useAgentContext();
  const input = useStore((s) => s.chatDraft);
  const setInput = useStore((s) => s.setChatDraft);
  const [lastTurnMs, setLastTurnMs] = useState<number | null>(null);
  const sendingRef = useRef(false);
  const askedRef = useRef<Message | null>(null);
  const stoppedRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const stuckRef = useRef(true);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const activitySteps = useActivity((s) => s.steps);
  const meta = useStore((s) => s.threadMeta);
  const markAsked = useStore((s) => s.markAsked);
  const clearScreen = useStore((s) => s.clearScreen);
  const loadThread = useStore((s) => s.loadThread);
  const connected = useStore((s) => s.connected);

  useEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    const ro = new ResizeObserver(() => {
      const el = scrollRef.current;
      if (stuckRef.current && el) el.scrollTop = el.scrollHeight;
    });
    ro.observe(content);
    return () => ro.disconnect();
  }, []);

  const onScroll = () => {
    const el = scrollRef.current;
    if (el) stuckRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  };

  const sendText = async (text: string, lookCloser?: string) => {
    if (isStreaming || sendingRef.current) return;
    sendingRef.current = true;
    stoppedRef.current = false;
    setLastTurnMs(null);
    const startedAt = performance.now();
    try {
      const userMsg: Message = { id: crypto.randomUUID(), role: "user", content: text };
      askedRef.current = userMsg;
      stuckRef.current = true;
      markAsked(userMsg.id, messages.length > 0);
      addMessage(userMsg);
      useActivity.getState().clear();
      agentClient.startNewRun();
      if (lookCloser) rerouteNextRun({ url: "/api/review/look-closer", body: { path: lookCloser } });
      await agentClient.runAgent(
        [...messages, userMsg],
        Object.values(toolDefs).map((t) => t.definition),
        agentSubscriber,
        getForwardedProps(),
      );
      // The server stored its own copy of the question under its own id; reload so the window boundary finds it.
      if (lookCloser) void loadThread(meta?.showing_all ?? false);
      // A stopped run ends with the question unanswered on purpose; that is not "no output".
      else if (!stoppedRef.current) setLastTurnMs(performance.now() - startedAt);
    } catch (err) {
      // A run that dies before the stream opens rejects instead of emitting RUN_ERROR.
      if (!isAbortError(err)) {
        addMessage({ id: `error_${Date.now()}`, role: "assistant", content: `Error: ${describeRunError(err)}` });
      }
    } finally {
      // A run that failed before its request must not send the next question to Look closer.
      if (lookCloser) rerouteNextRun(null);
      sendingRef.current = false;
    }
  };

  const latest = useRef({ messages, isStreaming });
  latest.current = { messages, isStreaming };
  useEffect(() => {
    setRestoredSink((proposalId, content) => {
      const { messages: msgs } = latest.current;
      const call = msgs.flatMap((m) => (m as unknown as { toolCalls?: ToolCallLike[] }).toolCalls ?? []).find((tc) => {
        try {
          return JSON.parse(tc.function?.arguments || "{}").proposal_id === proposalId;
        } catch {
          return false;
        }
      });
      if (!call) return;
      const answer = { id: `tool_${crypto.randomUUID()}`, role: "tool", toolCallId: call.id, content } as unknown as Message;
      addMessage(answer);
      agentClient.startNewRun();
      void agentClient
        .runAgent([...msgs, answer], Object.values(toolDefs).map((t) => t.definition), agentSubscriber, getForwardedProps())
        .catch((err) => {
          if (!isAbortError(err)) addMessage({ id: `error_${Date.now()}`, role: "assistant", content: `Error: ${describeRunError(err)}` });
        });
    });
    return () => setRestoredSink(null);
  });

  const outbox = useStore((s) => s.outbox);
  const takeOutbox = useStore((s) => s.takeOutbox);
  useEffect(() => {
    if (!outbox || isStreaming || sendingRef.current) return;
    takeOutbox(outbox.n);
    void sendText(outbox.text, outbox.lookCloser);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- sendText is recreated every render
  }, [outbox, isStreaming, takeOutbox]);

  const send = async () => {
    const text = input.trim();
    if (!text || isStreaming || sendingRef.current) return;
    setInput("");
    await sendText(text);
  };

  const stop = () => {
    const id = currentRunId();
    const asked = sendingRef.current ? askedRef.current : null;
    stoppedRef.current = true;
    terminateRun();
    // lm-ag-ui drops the run's question on terminate, but the server already stored it; keep the view in step.
    if (asked) addMessage(asked);
    if (id) void routes.cancelRun(id).catch(() => {});
  };

  const [stalled, setStalled] = useState(false);
  const [stallExtensions, setStallExtensions] = useState(0);
  useEffect(() => {
    setStalled(false);
    if (!isStreaming) return;
    const t = setTimeout(() => setStalled(true), limits.stallMs);
    return () => clearTimeout(t);
  }, [isStreaming, messages, currentMessage, activitySteps, limits.stallMs, stallExtensions]);
  const stallLabel = limits.stallMs >= 120_000 ? `${Math.round(limits.stallMs / 60_000)} minutes` : `${Math.round(limits.stallMs / 1000)} seconds`;

  const lastNonTool = [...messages].reverse().find((m) => m.role !== "tool");
  const lastCalls = (lastNonTool as unknown as { toolCalls?: ToolCallLike[] } | undefined)?.toolCalls;
  const waitingOnCard = !!lastCalls?.some((tc) => isProposeTool(tc.function?.name ?? getToolNameFromCallId(tc.id)));
  const turnEnd = classifySettledTurn(messages);
  const endedOnToolCalls = !isStreaming && turnEnd === "tool-calls" && !waitingOnCard;
  const producedNothing = !isStreaming && lastTurnMs != null && turnEnd === "no-output";
  const limitHit = useStore((s) => s.limitHit) && !isStreaming;
  const [dismissedId, setDismissedId] = useState<string | null>(null);
  const showContinue = (endedOnToolCalls || producedNothing || limitHit) && dismissedId !== lastNonTool?.id;
  const lastUserText = lastNonTool?.role === "user" && typeof lastNonTool.content === "string" ? lastNonTool.content : null;

  const callsBefore = useStore((s) => s.callsBefore);
  const toolData = useStore((s) => s.toolData);
  const exploreLink = useExploreLink();
  const ask = (text: string) => void sendText(text);
  const renderCall = (tc: ToolCallLike) => {
    const name = tc.function?.name ?? getToolNameFromCallId(tc.id);
    const tool = name ? toolDefs[name] : undefined;
    const data = toolData[tc.id];
    let args: Record<string, unknown> = {};
    try {
      args = JSON.parse(tc.function?.arguments || "{}");
    } catch {
      // partial args while streaming
    }
    const resultMsg = messages.find((x) => x.role === "tool" && (x as unknown as { toolCallId?: string }).toolCallId === tc.id);
    const result = typeof resultMsg?.content === "string" ? resultMsg.content : "";
    return (
      <div key={tc.id}>
        {tool?.renderer
          ? ((tool.renderer(args, result, () => {}, () => undefined, tool.configJson) as unknown as ReactNode) ?? null)
          : name && isCardData(data)
            ? <ToolCard name={name} args={args} result={result} data={data} onAsk={ask} />
            : name && <ToolChip id={tc.id} name={name} args={args} result={result} />}
        {data?.kind === "learned" &&
          data.items.map((l) => (
            <LearnedChip
              key={l.memory_id}
              text={l.text}
              url={l.url}
              onUndo={() => routes.undoWebLearned(l)}
              onConfirm={() => routes.confirmMemory(l.memory_id)}
              confirmed={l.confirmed}
            />
          ))}
      </div>
    );
  };

  // Server ids are unique, but a restored or replayed thread must not crash React on a repeat.
  const rendered = useMemo(() => {
    const seen = new Map<string, number>();
    return messages.map((m) => {
      const n = (seen.get(m.id) ?? 0) + 1;
      seen.set(m.id, n);
      return { m, key: n === 1 ? m.id : `${m.id}#${n}` };
    });
  }, [messages]);

  const renderMessage = (m: Message, key: string): ReactNode => {
    // suggest_next shows as pills under the answer (UI-FOLLOW-UPS), not as a chip.
    const toolCalls = (m as unknown as { toolCalls?: ToolCallLike[] }).toolCalls?.filter(
      (tc) => (tc.function?.name ?? getToolNameFromCallId(tc.id)) !== "suggest_next",
    );
    if (m.role === "assistant" && !toolCalls?.length && !(typeof m.content === "string" && m.content.trim())) return null;
    if (m.role === "assistant" && toolCalls?.length) {
      const before = callsBefore[m.id] ?? [];
      return (
        <div key={key} className="fade-up">
          {toolCalls.filter((tc) => before.includes(tc.id)).map(renderCall)}
          {typeof m.content === "string" && m.content && (
            <div className="break-words leading-relaxed">
              <Markdown text={m.content} />
            </div>
          )}
          {toolCalls.filter((tc) => !before.includes(tc.id)).map(renderCall)}
        </div>
      );
    }
    if (m.role === "tool") return null;
    if (m.role === "user") {
      return (
        <div key={key} className="text-right">
          <span className="inline-block max-w-[85%] break-words rounded-2xl rounded-br-md bg-brass/15 px-3 py-1.5 text-left text-paper">
            {typeof m.content === "string" ? m.content : "[attachment]"}
          </span>
        </div>
      );
    }
    const content = typeof m.content === "string" ? m.content : "";
    if (m.role === "assistant" && m.id.startsWith("error_")) {
      return (
        <div key={key} className="fade-up break-words rounded-xl border border-err/30 bg-err/10 px-3 py-2 text-[length:var(--fs-sm)] text-err">
          {content}
        </div>
      );
    }
    return (
      <div key={key} className="fade-up break-words leading-relaxed">
        <Markdown text={content} />
      </div>
    );
  };

  // The window can start at a tool answer, which renders nothing; the divider goes before the next visible message.
  const scrollback: ReactNode[] = [];
  let boundaryDue = false;
  for (const { m, key } of rendered) {
    if (meta?.has_hidden && m.id === meta.window_start_id) boundaryDue = true;
    const body = renderMessage(m, key);
    if (!body) continue;
    if (boundaryDue) {
      scrollback.push(<Boundary key={`boundary-${key}`} />);
      boundaryDue = false;
    }
    scrollback.push(body);
  }
  const boundaryAtEnd = boundaryDue || (!!meta?.has_hidden && meta.window_start_id == null);

  // A card decision (a client "tool_" answer) starts a new answer just like a question does.
  const asks = messages.map((m) => m.role === "user" || (m.role === "tool" && m.id.startsWith("tool_") && !m.id.startsWith("tool_result_")));
  const turnData = messages
    .slice(asks.lastIndexOf(true) + 1)
    .flatMap((m) => (m as unknown as { toolCalls?: ToolCallLike[] }).toolCalls ?? [])
    .map((tc) => toolData[tc.id])
    .filter((d) => d != null);
  const pills = !isStreaming && !waitingOnCard ? followUps(turnData, { explore: !!exploreLink.onOpenExplore }) : [];

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div ref={scrollRef} onScroll={onScroll} className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden">
        <div ref={contentRef} className="mx-auto w-full max-w-3xl space-y-3 px-4 py-4">
          {!!meta?.earlier && !isStreaming && (
            <div className="text-center">
              <button
                className="text-[length:var(--fs-xs)] font-semibold text-faint underline-offset-2 hover:text-paper hover:underline"
                onClick={() => void loadThread(!meta.showing_all)}
              >
                {meta.showing_all ? "Hide earlier chat" : `Show earlier chat (${meta.earlier} message${meta.earlier === 1 ? "" : "s"})`}
              </button>
            </div>
          )}
          {scrollback}
          <FollowUps items={pills} onAsk={ask} disabled={!connected} />
          {isStreaming && <LiveActivity />}
          {isStreaming && currentMessage && (
            <div className="break-words leading-relaxed">
              <Markdown text={currentMessage} streaming />
            </div>
          )}
          {isStreaming && stalled && (
            <Prompt
              actions={
                <>
                  <button className={btnAccent} onClick={() => setStallExtensions((n) => n + 1)}>
                    Keep waiting
                  </button>
                  <button className={btnQuiet} onClick={stop}>
                    Stop
                  </button>
                </>
              }
            >
              No response from the model for {stallLabel}.
            </Prompt>
          )}
          {!isStreaming && lastTurnMs != null && messages.length > 0 && <div className="text-[length:var(--fs-sm)] text-faint">{humanElapsed(lastTurnMs)}</div>}
          {showContinue && (
            <Prompt
              actions={
                <>
                  <button className={btnAccent} onClick={() => void sendText(producedNothing ? (lastUserText ?? "Continue.") : "Continue.")}>
                    {producedNothing ? "Try again" : "Keep going"}
                  </button>
                  <button className={btnQuiet} onClick={() => setDismissedId(lastNonTool?.id ?? null)}>
                    Dismiss
                  </button>
                </>
              }
            >
              {producedNothing
                ? "The model replied with no content. Check the provider's base URL and that its server is reachable (Settings, AI)."
                : limitHit
                  ? `Hoardini used its ${limits.maxSteps} tool steps for this turn before finishing.`
                  : `Hoardini stopped without a final answer. It may have hit the tool call limit (${limits.maxSteps}).`}
            </Prompt>
          )}
          {boundaryAtEnd && <Boundary />}
          {(meta ? meta.fresh : messages.length === 0) && !isStreaming && <Starters onAsk={(t) => void sendText(t)} />}
        </div>
      </div>
      <div className="border-t border-line-soft bg-ink-900 px-4 py-3">
        <div className="mx-auto flex w-full max-w-3xl items-end gap-2">
          <IconButton
            title="Clear the chat: Hoardini starts fresh, and earlier messages stay saved"
            aria-label="Clear chat"
            className="!h-11 !w-11 !rounded-full bg-brass text-on-accent hover:!bg-brass hover:opacity-90"
            disabled={isStreaming || !connected}
            onClick={() => void clearScreen().catch(() => {})}
          >
            <EraserIcon />
          </IconButton>
          <textarea
            ref={inputRef}
            className="max-h-40 min-h-[44px] flex-1 resize-none rounded-2xl border border-line bg-ink-850 px-3.5 py-2.5 text-paper placeholder:text-faint focus:border-brass focus:outline-none"
            rows={1}
            placeholder="Ask what is taking up space..."
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void send();
              }
            }}
          />
          {isStreaming ? (
            <IconButton title="Stop" aria-label="Stop the run" className="!h-11 !w-11 !rounded-full bg-err/15 text-err hover:!bg-err/25" onClick={stop}>
              <StopIcon />
            </IconButton>
          ) : (
            <IconButton
              title="Send"
              aria-label="Send message"
              className="!h-11 !w-11 !rounded-full bg-brass text-on-accent hover:!bg-brass hover:opacity-90"
              onClick={() => void send()}
              disabled={!input.trim()}
            >
              <SendIcon />
            </IconButton>
          )}
        </div>
      </div>
    </div>
  );
}

export function ChatSession({ seed }: { seed: Message[] | null }) {
  const setChatBusy = useStore((s) => s.setChatBusy);
  const refreshThreadMeta = useStore((s) => s.refreshThreadMeta);
  const [err, setErr] = useState<string | null>(null);
  const [limits, setLimits] = useState({ stallMs: resolveStallMs(null), maxSteps: resolveMaxSteps(null) });
  const drawer = useStore((s) => s.drawer);
  const tools = useMemo(() => proposalTools(), []);

  const agent = useAgent({
    baseUrl: "/api",
    agentId: "run",
    tools,
    tokenProvider: async () => getToken(),
    requestHandler,
    sendFullHistory: false,
    initialThreadId: THREAD_ID,
    idleTimeoutMs: WATCHDOG_BACKSTOP_MS,
    safetyTimeoutMs: WATCHDOG_BACKSTOP_MS,
    pruneOutboundMessages: pruneOutbound,
    onError: (e) => setErr(e.message),
    onLifecycleEvent: (e) => useActivity.getState().onLifecycle(e),
  });

  const seeded = useRef(false);
  useEffect(() => {
    if (!seeded.current && seed?.length) agent.setMessages(seed);
    seeded.current = true;
    // eslint-disable-next-line react-hooks/exhaustive-deps -- mount-only; the component is keyed by thread
  }, []);

  const wasStreaming = useRef(false);
  useEffect(() => {
    setChatBusy(agent.isStreaming);
    // A finished run may have trimmed the window or crossed the session gap.
    if (wasStreaming.current && !agent.isStreaming) void refreshThreadMeta();
    wasStreaming.current = agent.isStreaming;
  }, [agent.isStreaming, setChatBusy, refreshThreadMeta]);
  useEffect(() => () => setChatBusy(false), [setChatBusy]);

  useEffect(() => {
    if (drawer !== null) return;
    routes
      .providers()
      .then((ps: ProviderView[]) => {
        const p = ps.find((x) => x.is_active);
        setLimits({ stallMs: resolveStallMs(p?.timeout_s), maxSteps: resolveMaxSteps(p?.max_tool_steps) });
      })
      .catch(() => {});
  }, [drawer]);

  return (
    <AgentProvider value={agent}>
      {err && (
        <div className="flex items-start gap-2 border-b border-err/30 bg-err/10 px-4 py-2 text-[length:var(--fs-sm)] text-err">
          <span className="flex-1 break-words font-mono">{err}</span>
          <IconButton size="sm" title="Dismiss" aria-label="Dismiss error" className="text-muted hover:text-paper" onClick={() => setErr(null)}>
            ✕
          </IconButton>
        </div>
      )}
      <Chat limits={limits} />
    </AgentProvider>
  );
}
