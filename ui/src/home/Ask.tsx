import { useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { useAiSetup } from "../aiSetup";
import { getToken, readSse } from "../api/client";
import { Markdown } from "../components/Markdown";
import { useStore } from "../store";

type Turn = { id: string; role: "user" | "assistant"; text: string };
type Stored = { id?: string; role?: string; content?: unknown };

const SHOWN_TURNS = 6;

// A short stable ID per path; the server keeps one conversation per item (AGENT-ASK).
export function threadFor(path: string): string {
  let h1 = 0x811c9dc5;
  let h2 = 7;
  for (const ch of path.toLowerCase()) {
    const c = ch.codePointAt(0)!;
    h1 = Math.imul(h1 ^ c, 0x01000193) >>> 0;
    h2 = (Math.imul(h2, 31) + c) >>> 0;
  }
  return `item:${h1.toString(36)}${h2.toString(36)}`;
}

// The first question carries the item it is about; the screen shows only what was typed.
const shown = (text: string) => text.replace(/^About [^\n]*?\): /, "");

async function load(id: string): Promise<Turn[]> {
  const t = getToken();
  const res = await fetch(`/api/thread?id=${encodeURIComponent(id)}`, { headers: t ? { Authorization: `Bearer ${t}` } : {} });
  if (!res.ok) return [];
  const body = (await res.json()) as { messages?: Stored[] };
  return (body.messages ?? [])
    .filter((m): m is Stored & { role: "user" | "assistant"; content: string } => (m.role === "user" || m.role === "assistant") && typeof m.content === "string" && m.content.trim() !== "")
    .map((m, i) => ({ id: m.id ?? `m${i}`, role: m.role, text: m.role === "user" ? shown(m.content) : m.content }));
}

export type AskHandle = { ask: (q: string) => void };

export function Ask({ path, name, ref }: { path: string; name: string; ref?: Ref<AskHandle> }) {
  const id = threadFor(path);
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [looking, setLooking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ctl = useRef<AbortController | null>(null);
  const setChatBusy = useStore((s) => s.setChatBusy);

  useEffect(() => {
    let live = true;
    load(id).then((t) => live && setTurns(t), () => {});
    return () => {
      live = false;
      ctl.current?.abort();
    };
  }, [id]);
  useEffect(() => () => setChatBusy(false), [setChatBusy]);

  const send = async (text: string) => {
    const q = text.trim();
    if (!q || busy) return;
    setError(null);
    const first = turns.length === 0;
    const userTurn: Turn = { id: crypto.randomUUID(), role: "user", text: q };
    const answer: Turn = { id: crypto.randomUUID(), role: "assistant", text: "" };
    setTurns((t) => [...t, userTurn, answer]);
    setBusy(true);
    setChatBusy(true);
    const c = new AbortController();
    ctl.current = c;
    try {
      const t = getToken();
      const res = await fetch("/api/agent/run", {
        method: "POST",
        signal: c.signal,
        headers: { "Content-Type": "application/json", Accept: "text/event-stream", ...(t ? { Authorization: `Bearer ${t}` } : {}) },
        body: JSON.stringify({
          threadId: id,
          runId: crypto.randomUUID(),
          messages: [{ id: userTurn.id, role: "user", content: first ? `About ${path} (${name}): ${q}` : q }],
          tools: [],
          context: [],
          state: {},
          forwardedProps: {},
        }),
      });
      if (!res.ok || !res.body) throw new Error((await res.text().catch(() => "")) || res.statusText);
      await readSse(res.body, (data) => {
        let e: { type?: string; delta?: string; message?: string; name?: string; value?: { seconds?: number } };
        try {
          e = JSON.parse(data);
        } catch {
          return;
        }
        if (e.type === "TEXT_MESSAGE_CONTENT" && e.delta) {
          setLooking(false);
          setTurns((ts) => ts.map((x) => (x.id === answer.id ? { ...x, text: x.text + e.delta } : x)));
        } else if (e.type === "TOOL_CALL_START") setLooking(true);
        else if (e.type === "RUN_ERROR") setError(e.message ?? "Hoardini couldn't answer.");
        else if (e.type === "CUSTOM" && e.name === "rate_limit_wait" && typeof e.value?.seconds === "number")
          useAiSetup.getState().setWait(Date.now() + e.value.seconds * 1000);
      });
    } catch (e) {
      if (!c.signal.aborted) setError(e instanceof Error ? e.message : String(e));
    } finally {
      setTurns((ts) => ts.filter((x) => x.id !== answer.id || x.text.trim() !== ""));
      setBusy(false);
      setLooking(false);
      setChatBusy(false);
      useAiSetup.getState().setWait(0);
    }
  };
  useImperativeHandle(ref, () => ({ ask: (q) => void send(q) }));

  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-2 border-t border-line-soft pt-3">
      {turns.slice(-SHOWN_TURNS).map((t, i, all) =>
        t.role === "user" ? (
          <p key={t.id} className="text-[length:var(--fs-sm)] font-extrabold [overflow-wrap:anywhere]">
            {t.text}
          </p>
        ) : (
          <div key={t.id} className="text-[length:var(--fs-sm)] text-paper-dim">
            <Markdown text={t.text} streaming={busy && i === all.length - 1} />
          </div>
        ),
      )}
      {busy && <p className="pulse-soft text-[length:var(--fs-sm)] text-muted">{looking ? "Looking inside…" : "Thinking…"}</p>}
      {error && <p className="text-[length:var(--fs-sm)] text-err">{error}</p>}
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (busy) return;
          void send(draft);
          setDraft("");
        }}
      >
        <input
          className="min-w-0 flex-1 rounded-full bg-ink-850 px-3.5 py-2 text-[length:var(--fs-sm)] text-paper placeholder:text-faint focus:outline focus:outline-1 focus:outline-brass"
          placeholder="Ask about this…"
          value={draft}
          disabled={busy}
          onChange={(e) => setDraft(e.target.value)}
          aria-label={`Ask about ${name}`}
        />
        {busy ? (
          <button type="button" className="text-[length:var(--fs-sm)] font-bold text-muted hover:text-paper" onClick={() => ctl.current?.abort()}>
            Stop
          </button>
        ) : null}
      </form>
    </div>
  );
}
