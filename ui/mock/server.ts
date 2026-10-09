// Mock of the Hoardini loopback API (hoardini-impl.md) with canned data; token "dev", port 7043.
import type * as T from "../src/api/types";

const PORT = Number(process.env.MOCK_PORT ?? 7043);
const TOKEN = "dev";
const GB = 1024 ** 3;
const MB = 1024 ** 2;
const now = () => Date.now();
const uuid = () => crypto.randomUUID();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// Every fixed drive; status.volumes is the watched subset (GET/PUT /api/volumes).
const drives: T.VolumeView[] = [
  { root: "C:\\", label: "Windows", fs: "NTFS", total_bytes: 1906 * GB, free_bytes: 62 * GB, watched: true, selected: true, connected: true, has_journal: true },
  { root: "D:\\", label: "Data", fs: "NTFS", total_bytes: 931 * GB, free_bytes: 410 * GB, watched: true, selected: false, connected: true, has_journal: true },
  { root: "E:\\", label: "Camera", fs: "exFAT", total_bytes: 238 * GB, free_bytes: 21 * GB, watched: false, selected: false, connected: true, has_journal: false },
  { root: "F:\\", label: "Old backup", fs: "NTFS", total_bytes: 3726 * GB, free_bytes: 1210 * GB, watched: true, selected: false, connected: false, has_journal: true },
];
const watchedDrives = () => drives.filter((d) => d.watched);
const selectedDrive = () => drives.find((d) => d.selected) ?? drives[0]!;

const status: T.Status = {
  volumes: watchedDrives(),
  index: {
    state: "refreshing",
    mode: "background",
    progress: { phase: "walk", dirs: 120_000, files: 2_300_000, bytes: 700 * GB, fraction: 0.4 },
    last_refresh_ms: now() - 3 * 86_400_000,
    generation: 12,
  },
  jobs: [],
  vanished_total_bytes: Math.round(52.4 * GB),
  active_provider: { id: "prov-lmstudio", name: "LM Studio", model: "qwen3.8-27b" },
  running_runs: 0,
};

const setFill = (f: number) => {
  const c = selectedDrive();
  c.free_bytes = Math.round(c.total_bytes * (1 - Math.min(1, Math.max(0, f))));
};
if (process.env.MOCK_FILL) setFill(Number(process.env.MOCK_FILL));

const jobs = new Map<string, T.Job>();
const subscribers = new Set<(frame: string) => void>();
let dirty = true;
const emit = (obj: unknown) => subscribers.forEach((s) => s(`data: ${JSON.stringify(obj)}\n\n`));
const emitJob = (j: T.Job) => emit({ type: "job", ...j });

setInterval(() => {
  if (!dirty) return;
  dirty = false;
  status.jobs = [...jobs.values()].sort((a, b) => b.created_ms - a.created_ms);
  emit({ type: "status", ...status });
}, 500);
setInterval(() => subscribers.forEach((s) => s(": ping\n\n")), 15_000);

let refreshTimer: ReturnType<typeof setInterval> | null = null;
function startRefresh(mode: "background" | "fast", from = 0) {
  status.index.state = "refreshing";
  status.index.mode = mode;
  let f = from;
  const step = mode === "fast" ? 0.08 : 0.012;
  if (refreshTimer) clearInterval(refreshTimer);
  refreshTimer = setInterval(() => {
    f = Math.min(1, f + step);
    status.index.progress = { phase: f < 0.15 ? "hotspots" : f < 0.9 ? "walk" : "finalize", dirs: Math.round(f * 300_000), files: Math.round(f * 5_800_000), bytes: Math.round(f * 1700 * GB), fraction: f };
    dirty = true;
    if (f >= 1) {
      clearInterval(refreshTimer!);
      refreshTimer = null;
      status.index.state = "idle";
      status.index.mode = null;
      status.index.progress = null;
      status.index.last_refresh_ms = now();
      status.index.generation += 1;
    }
  }, 700);
  dirty = true;
}
if (!process.env.MOCK_IDLE) startRefresh("background", 0.4);
else {
  status.index.state = "idle";
  status.index.mode = null;
  status.index.progress = null;
}

const summary: T.IndexSummary = {
  age_ms: 3 * 86_400_000,
  free_delta_bytes: -23 * GB,
  movers: [
    { path: "~\\.lmstudio", delta_bytes: 16 * GB },
    { path: "~\\Downloads", delta_bytes: 5 * GB },
    { path: "~\\.cache", delta_bytes: 1.5 * GB },
  ],
  duplicate_hint: { bytes: 27 * GB, count: 1 },
};

const memories: T.Memory[] = [
  { id: 1, scope: "path", key: "~\\.lmstudio\\models\\qwen3.8-27b", kind: "protect", text: "Keep Qwen3.8 27B; it is my daily driver.", source: "chat 2026-09-12", confidence: 0.95, created_ms: now() - 26 * 86_400_000, updated_ms: now() - 26 * 86_400_000, review_after_ms: null, pinned: true, last_used_ms: now() - 2 * 86_400_000, forgotten_ms: null },
  { id: 2, scope: "glob", key: "**\\node_modules", kind: "verdict", text: "Safe to delete when the project has not been touched in 60 days; npm install brings it back.", source: "web: https://docs.npmjs.com/cli/commands/npm-install", confidence: 0.7, created_ms: now() - 9 * 86_400_000, updated_ms: now() - 9 * 86_400_000, review_after_ms: null, pinned: false, last_used_ms: now() - 5 * 86_400_000, forgotten_ms: null },
  { id: 3, scope: "global", key: "", kind: "preference", text: "Prefer archiving to the NAS over deleting when a file is bigger than 10 GB.", source: "chat 2026-10-01", confidence: null, created_ms: now() - 7 * 86_400_000, updated_ms: now() - 7 * 86_400_000, review_after_ms: null, pinned: false, last_used_ms: now() - 7 * 86_400_000, forgotten_ms: now() - 3 * 86_400_000 },
];
let memLimit = 200;
let memSeq = 10;

const providers: T.ProviderView[] = [
  { id: "prov-lmstudio", name: "LM Studio", base_url: "http://127.0.0.1:1234/v1", model: "qwen3.8-27b", api_key: null, context_window: 32768, timeout_s: 180, max_tool_steps: 20, small: false, params: { temperature: 0.2 }, headers: {}, extra_body: {}, is_active: true, has_key: false, caps: { tools: true, streaming: true, streamed_tool_calls: true, tool_choice: true } },
];
const presets: T.Preset[] = [
  { id: "cerebras", name: "Cerebras", base_url: "https://api.cerebras.ai/v1", needs_key: true, default_model: "gpt-oss-120b", key_url: "https://cloud.cerebras.ai/platform/" },
  { id: "gemini", name: "Google Gemini", base_url: "https://generativelanguage.googleapis.com/v1beta/openai/", needs_key: true, default_model: "gemini-3.8-flash", key_url: "https://aistudio.google.com/apikey" },
  { id: "openrouter", name: "OpenRouter", base_url: "https://openrouter.ai/api/v1", needs_key: true, default_model: "nvidia/nemotron-3-super-120b-a12b:free", key_url: "https://openrouter.ai/settings/keys" },
  { id: "lmstudio", name: "LM Studio", base_url: "http://127.0.0.1:1234/v1", needs_key: false, default_model: null, key_url: null },
  { id: "ollama", name: "Ollama", base_url: "http://127.0.0.1:11434/v1", needs_key: false, default_model: null, key_url: null },
  { id: "openai", name: "OpenAI", base_url: "https://api.openai.com/v1", needs_key: true, default_model: null, key_url: null },
];
let setupDone = true;
const secrets = new Set<string>();
const serverModels = ["qwen3.8-27b", "llama-3.3-70b-instruct", "mistral-small-24b", "gemma-4-12b"];

const destinations: T.Destination[] = [{ id: "dest-nas", name: "NAS", kind: "smb", config: { path: "\\\\nas\\backups\\hoardini", username: "josh" }, created_ms: now() - 20 * 86_400_000 }];

const searchProviders: T.SearchProviderView[] = (["tavily", "brave", "exa", "serper", "searxng"] as T.SearchKind[]).map((kind) => ({ kind, is_active: kind === "tavily", has_key: kind === "tavily", base_url: null }));

const history: T.HistoryPoint[] = Array.from({ length: 14 }, (_, i) => {
  const free = (140 - i * 6 - (i > 9 ? (i - 9) * 4 : 0) + (i % 4 === 3 ? 9 : 0)) * GB;
  return { finished_ms: now() - (14 - i) * 2 * 86_400_000, total_bytes: 1906 * GB, free_bytes: free, used_bytes: 1906 * GB - free, indexed_bytes: 1906 * GB - free - 20 * GB };
});

const settings: T.Settings = { index_speed: "lean", builtin_idle_minutes: 10, index_size_bytes: 184 * MB };

type Thread = { updated_ms: number; messages: ({ id: string } & Record<string, unknown>)[]; proposals: Record<string, T.Proposal>; windowStart: number };
const thread: Thread = { updated_ms: 0, messages: [], proposals: {}, windowStart: 0 };
const SESSION_GAP_MS = 6 * 3_600_000;
const proposals = new Map<string, T.Proposal>();
const cancelled = new Set<string>();

function threadDetail() {
  const last = thread.messages.length ? thread.updated_ms : null;
  const start = thread.messages[thread.windowStart]?.id ?? null;
  return {
    id: "main",
    messages: thread.messages,
    proposals: thread.proposals,
    window_start_id: start,
    has_hidden: thread.windowStart > 0,
    fresh: last == null || now() - last > SESSION_GAP_MS || start == null,
  };
}

function makeProposal(kind: T.ProposalKind): T.Proposal {
  const mk = (id: number, path: string, alloc: number, p: Partial<T.ProposalItem>): T.ProposalItem => ({ id, path, kind: "file", alloc, verdict: "redownload", cap: null, reason: null, block_reason: null, selected: true, status: "pending", detail: null, ...p });
  const items: T.ProposalItem[] = [
    mk(1, "C:\\Users\\josha\\.lmstudio\\models\\llama-3.3-70b-Q4_K_M.gguf", 42.5 * GB, { reason: "Public model on Hugging Face; last opened 74 days ago." }),
    mk(2, "C:\\Users\\josha\\.lmstudio\\models\\mistral-small-24b-Q6_K.gguf", 9.9 * GB, { verdict: "review", cap: "review", reason: "Model said safe, but it was opened 12 days ago." }),
    mk(3, "C:\\Users\\josha\\.lmstudio\\models\\qwen3.8-27b-Q8_0.gguf", 27.1 * GB, { verdict: "keep", selected: false, reason: "You said to keep this one (Sep 12)." }),
    mk(4, "C:\\Users\\josha\\.lmstudio\\models\\gemma-4-12b-Q8_0.gguf", 12.4 * GB, { verdict: "redownload", selected: false, block_reason: "Open in LM Studio.exe (pid 8124)", status: "blocked" }),
  ];
  const p: T.Proposal = {
    id: uuid(),
    kind,
    status: "pending",
    total_alloc: items.filter((i) => i.selected).reduce((a, i) => a + i.alloc, 0),
    created_ms: now(),
    job_id: null,
    destination: kind === "archive" ? { id: "dest-nas", name: "NAS" } : null,
    format: null,
    summary: kind === "archive" ? "Copy two models to NAS, verify, then delete the originals." : "Two models you have not opened in weeks.",
    items,
  };
  proposals.set(p.id, p);
  return p;
}

// Set through POST /api/mock {"next_job": ...}; shapes the next approval for checking the mascot's moves.
type NextJob = { kind?: T.ProposalKind; outcome?: "failed" | "cancelled"; ms?: number; cloud?: boolean };
let nextJob: NextJob | null = null;

function runJob(p: T.Proposal, itemIds: number[]): T.Job {
  const plan = nextJob;
  nextJob = null;
  if (plan?.kind) p.kind = plan.kind;
  if (plan?.cloud) {
    let d = destinations.find((x) => x.kind === "rclone");
    if (!d) destinations.push((d = { id: "dest-drive", name: "Drive", kind: "rclone", config: {}, created_ms: now() }));
    p.destination = { id: d.id, name: d.name };
  }
  const chosen = p.items.filter((i) => itemIds.includes(i.id) && !i.block_reason);
  const total = chosen.reduce((a, i) => a + i.alloc, 0);
  const job: T.Job = { id: uuid(), kind: p.kind, proposal_id: p.id, status: "running", progress: { done_bytes: 0, total_bytes: total, done_items: 0, total_items: chosen.length, current: chosen[0]?.path ?? null }, freed_bytes: 0, error: null, created_ms: now(), finished_ms: null };
  jobs.set(job.id, job);
  p.job_id = job.id;
  p.status = "running";
  for (const i of p.items) i.selected = chosen.includes(i);
  emitJob(job);
  dirty = true;
  void (async () => {
    let done = 0;
    for (let n = 0; n < chosen.length; n++) {
      const it = chosen[n]!;
      for (let s = 1; s <= 5; s++) {
        await sleep(plan?.ms ? plan.ms / 5 / chosen.length : 450);
        job.progress.done_bytes = done + (it.alloc * s) / 5;
        job.progress.current = it.path;
        emitJob(job);
      }
      done += it.alloc;
      it.status = "done";
      job.progress.done_items = n + 1;
    }
    if (plan?.outcome) {
      Object.assign(job, { status: plan.outcome, error: plan.outcome === "failed" ? "Access is denied" : null, finished_ms: now() });
      p.status = "failed";
      emitJob(job);
      dirty = true;
      return;
    }
    job.status = "done";
    job.freed_bytes = total;
    job.finished_ms = now();
    p.status = "done";
    status.vanished_total_bytes += total;
    selectedDrive().free_bytes += total;
    emitJob(job);
    dirty = true;
  })();
  return job;
}

// ---- AG-UI run ----
const enc = new TextEncoder();
type Sse = { send: (e: Record<string, unknown>) => void; closed: () => boolean };

async function say(s: Sse, text: string) {
  const messageId = uuid();
  s.send({ type: "TEXT_MESSAGE_START", messageId, role: "assistant" });
  for (const chunk of text.match(/[\s\S]{1,18}(\s|$)/g) ?? [text]) {
    if (s.closed()) return;
    s.send({ type: "TEXT_MESSAGE_CONTENT", messageId, delta: chunk });
    await sleep(35);
  }
  s.send({ type: "TEXT_MESSAGE_END", messageId });
}

async function readTool(s: Sse, name: string, args: unknown, result: string, ms = 600) {
  const toolCallId = uuid();
  const parentMessageId = uuid();
  s.send({ type: "TOOL_CALL_START", toolCallId, toolCallName: name, parentMessageId });
  s.send({ type: "TOOL_CALL_ARGS", toolCallId, delta: JSON.stringify(args) });
  s.send({ type: "TOOL_CALL_END", toolCallId });
  await sleep(ms);
  s.send({ type: "TOOL_CALL_RESULT", messageId: uuid(), toolCallId, content: result, role: "tool" });
  return toolCallId;
}

async function agentRun(input: { runId: string; messages: { id?: string; role: string; content?: unknown }[] }, s: Sse) {
  const { runId } = input;
  const threadId = "main";
  const last = input.messages[input.messages.length - 1];
  const started = now();
  status.running_runs += 1;
  dirty = true;
  s.send({ type: "RUN_STARTED", threadId, runId });
  try {
    if (last?.role === "tool") {
      let approved = false;
      try {
        approved = !!JSON.parse(String(last.content)).approved;
      } catch {
        // non-JSON tool content is treated as declined
      }
      await sleep(400);
      await say(s, approved ? "Started. Watch the card and the header: the free-space bar will move as files go. I'll look at the result next time you ask." : "No problem. Nothing was changed.");
    } else {
      const text = String(last?.content ?? "");
      if (thread.messages.length && now() - thread.updated_ms > SESSION_GAP_MS) thread.windowStart = thread.messages.length;
      thread.messages.push({ id: last?.id ?? uuid(), role: "user", content: text });
      if (/^remember|remember that/i.test(text)) {
        await say(s, "Got it.");
        const toolCallId = await readTool(s, "remember", { scope: "global", kind: "preference", text }, JSON.stringify({ id: ++memSeq }), 300);
        memories.push({ id: memSeq, scope: "global", key: "", kind: "preference", text: text.replace(/^remember( that)?/i, "").trim() || text, source: "chat", confidence: null, created_ms: now(), updated_ms: now(), review_after_ms: null, pinned: false, last_used_ms: now(), forgotten_ms: null });
        s.send({ type: "CUSTOM", name: "memory_noted", value: { memory_id: memSeq, text: text.replace(/^remember( that)?/i, "").trim() || text, tool_call_id: toolCallId, pinned: false } });
      } else {
        await say(s, "Let me look at what is in `~\\.lmstudio`.\n\n");
        await readTool(s, "top_dirs", { under: "~\\.lmstudio", depth: 2, limit: 10 }, "~\\.lmstudio 96.1 GB\n  models 95.4 GB (9 files)\n  cache 0.6 GB", 700);
        await readTool(s, "inspect", { path: "~\\.lmstudio\\models\\llama-3.3-70b-Q4_K_M.gguf" }, "GGUF llama 70B Q4_K_M, 42.5 GB, last access 74 days ago (medium confidence), public on Hugging Face.", 900);
        await say(s, "**9 models, 96 GB.** On Sep 12 you said to keep Qwen3.8 27B, so I left it out. Two others have not been opened in a while:\n\n- `llama-3.3-70b` (42.5 GB), 74 days since last access\n- `mistral-small-24b` (9.9 GB), 12 days\n\nHere is a plan. Nothing happens until you approve it.\n\n");
        const kind: T.ProposalKind = /archive|nas/i.test(text) ? "archive" : "delete";
        const p = makeProposal(kind);
        thread.proposals[p.id] = p;
        const toolCallId = uuid();
        const name = `propose_${kind}`;
        const args = { proposal_id: p.id, paths: p.items.filter((i) => i.selected).map((i) => i.path), ...(kind === "archive" ? { destination: "NAS" } : {}) };
        s.send({ type: "TOOL_CALL_START", toolCallId, toolCallName: name, parentMessageId: uuid() });
        s.send({ type: "TOOL_CALL_ARGS", toolCallId, delta: JSON.stringify(args) });
        s.send({ type: "TOOL_CALL_END", toolCallId });
        s.send({ type: "CUSTOM", name: "proposal", value: { proposal_id: p.id, tool_call_id: toolCallId } });
        thread.messages.push({ id: uuid(), role: "assistant", content: "", toolCalls: [{ id: toolCallId, type: "function", function: { name, arguments: JSON.stringify(args) } }] });
      }
    }
    s.send({ type: "CUSTOM", name: "run_stats", value: { elapsed_ms: now() - started, steps: 3 } });
    thread.updated_ms = now();
    s.send({ type: "RUN_FINISHED", threadId, runId });
  } finally {
    status.running_runs = Math.max(0, status.running_runs - 1);
    dirty = true;
  }
}

// ---- HTTP ----
const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status, headers: { "content-type": "application/json" } });
const err = (message: string, code = 400) => json({ error: message }, code);

function sseResponse(start: (s: Sse) => Promise<void> | void, onCancel?: () => void) {
  let closed = false;
  const stream = new ReadableStream({
    async start(controller) {
      const s: Sse = {
        send: (e) => !closed && controller.enqueue(enc.encode(`data: ${JSON.stringify(e)}\n\n`)),
        closed: () => closed,
      };
      try {
        await start(s);
      } finally {
        if (!closed) controller.close();
      }
    },
    cancel() {
      closed = true;
      onCancel?.();
    },
  });
  return new Response(stream, { headers: { "content-type": "text/event-stream", "cache-control": "no-cache" } });
}

function testReport(p: T.ProviderView): T.TestReport {
  const found = !p.model || serverModels.includes(p.model);
  const suggestions = found || !p.model ? [] : serverModels.filter((m) => m.split("-")[0] === p.model!.split("-")[0] || m.includes(p.model!.slice(0, 4)));
  return { ok: true, error: null, models: serverModels, model_found: p.model ? found : null, suggestions, caps: { tools: true, streaming: true, streamed_tool_calls: !p.small, tool_choice: true }, latency_ms: 42 };
}

async function route(req: Request): Promise<Response> {
  const url = new URL(req.url);
  const path = url.pathname;
  const method = req.method;
  if (!path.startsWith("/api/")) return new Response("mock server: use the vite dev server for the UI", { status: 404 });
  const auth = req.headers.get("authorization");
  if (auth !== `Bearer ${TOKEN}` && url.searchParams.get("token") !== TOKEN) return err("unauthorized", 401);
  const body = async <B,>() => (await req.json().catch(() => ({}))) as B;
  const seg = path.slice(5).split("/");

  switch (seg[0]) {
    case "mock": {
      const b = await body<{ fill?: number; running_runs?: number; idle?: boolean; next_job?: NextJob }>();
      if (b.next_job) nextJob = b.next_job;
      if (b.fill != null) setFill(b.fill);
      if (b.running_runs != null) status.running_runs = b.running_runs;
      if (b.idle) Object.assign(status.index, { state: "idle", mode: null, progress: null });
      dirty = true;
      return json(status);
    }
    case "status":
      return json(status);
    case "index":
      if (seg[1] === "summary") return json(summary);
      if (seg[1] === "refresh") {
        if (status.index.state !== "idle") return json({ started: false });
        startRefresh(settings.index_speed === "fast" ? "fast" : "background");
        return json({ started: true });
      }
      break;
    case "agent":
      if (seg[1] === "run" && method === "POST") {
        const input = await body<{ runId: string; messages: { id?: string; role: string; content?: unknown }[] }>();
        return sseResponse((s) => agentRun(input, s), () => cancelled.add(input.runId));
      }
      if (seg[1] === "runs") return json({ ok: true });
      break;
    case "thread":
      if (seg[1] === "fresh" && method === "POST") {
        thread.windowStart = thread.messages.length;
        return json({ ok: true });
      }
      if (method === "DELETE") {
        if (status.running_runs > 0) return err("a chat run is in progress; stop it first", 409);
        thread.messages = [];
        thread.windowStart = 0;
        return json({ ok: true });
      }
      return json(threadDetail());
    case "proposals": {
      const p = proposals.get(seg[1]!);
      if (!p) return err("proposal not found", 404);
      if (!seg[2]) return json(p);
      if (seg[2] === "approve") {
        if (p.status !== "pending") return err(`proposal is ${p.status}`, 409);
        const { item_ids } = await body<{ item_ids: number[] }>();
        if (!item_ids?.length) return err("nothing selected");
        p.status = "approved";
        const job = runJob(p, item_ids);
        return json({ approved: true, items: item_ids, job_id: job.id });
      }
      if (seg[2] === "decline") {
        if (p.status !== "pending") return err(`proposal is ${p.status}`, 409);
        p.status = "declined";
        return json({ approved: false });
      }
      break;
    }
    case "jobs":
      if (!seg[1]) return json([...jobs.values()]);
      return json({ ok: true });
    case "memory": {
      if (seg[1] === "limit") {
        if (method === "PUT") memLimit = Math.max(10, (await body<{ limit: number }>()).limit);
        return json({ limit: memLimit });
      }
      if (!seg[1]) {
        if (method === "POST") {
          const b = await body<Omit<T.Memory, "id">>();
          const m: T.Memory = { id: ++memSeq, scope: b.scope, key: b.key, kind: b.kind, text: b.text, source: "you", confidence: null, created_ms: now(), updated_ms: now(), review_after_ms: null, pinned: true, last_used_ms: now(), forgotten_ms: null };
          memories.push(m);
          return json(m);
        }
        const q = url.searchParams.get("q")?.toLowerCase();
        const rows = memories.filter((m) => !q || `${m.text} ${m.key}`.toLowerCase().includes(q));
        return json([...rows.filter((m) => !m.forgotten_ms).sort((a, b) => (b.last_used_ms ?? 0) - (a.last_used_ms ?? 0)), ...rows.filter((m) => m.forgotten_ms)]);
      }
      const idx = memories.findIndex((m) => m.id === Number(seg[1]));
      if (idx < 0) return err("not found", 404);
      if (method === "DELETE") {
        memories.splice(idx, 1);
        return json({ ok: true });
      }
      if (seg[2] === "restore") {
        Object.assign(memories[idx]!, { forgotten_ms: null, last_used_ms: now() });
        return json(memories[idx]);
      }
      Object.assign(memories[idx]!, await body<Partial<T.Memory>>(), { updated_ms: now(), last_used_ms: now() });
      return json(memories[idx]);
    }
    case "setup":
      if (seg[1] === "done") {
        setupDone = true;
        return json({ ok: true });
      }
      return json({ done: setupDone });
    case "providers": {
      if (seg[1] === "presets") return json(presets);
      if (seg[1] === "local") return json([{ id: "lmstudio", name: "LM Studio", base_url: "http://127.0.0.1:1234/v1", models: serverModels }]);
      if (!seg[1]) {
        if (method === "POST") {
          const b = await body<T.ProviderConfig>();
          const v: T.ProviderView = { ...b, id: `prov-${uuid().slice(0, 8)}`, is_active: providers.length === 0, has_key: !!b.api_key && secrets.has(b.api_key), caps: null };
          providers.push(v);
          return json(v);
        }
        return json(providers);
      }
      const p = providers.find((x) => x.id === seg[1]);
      if (!p) return err("not found", 404);
      if (seg[2] === "activate") {
        providers.forEach((x) => (x.is_active = x === p));
        status.active_provider = { id: p.id, name: p.name, model: p.model };
        dirty = true;
        return json({ ok: true });
      }
      if (seg[2] === "test") {
        const r = testReport(p);
        p.caps = r.caps;
        return json(r);
      }
      if (method === "DELETE") {
        providers.splice(providers.indexOf(p), 1);
        return json({ ok: true });
      }
      if (method === "PUT") {
        const b = await body<T.ProviderConfig>();
        Object.assign(p, b, { id: p.id, has_key: !!b.api_key && secrets.has(b.api_key) });
        return json(p);
      }
      break;
    }
    case "secrets": {
      const { ref } = await body<{ ref: string }>();
      secrets.add(ref);
      return json({ ok: true });
    }
    case "runtime":
      return json({ installed: true, version: "1.75.1", size: 31_488_864, license_url: "https://github.com/rclone/rclone/blob/v1.75.1/COPYING" });
    case "destinations": {
      if (seg[1] === "rclone" && seg[2] === "authorize") return json({ token_ref: `secret://rclone/token/${uuid().slice(0, 8)}` });
      if (!seg[1]) {
        if (method === "POST") {
          const b = await body<T.DestinationInput>();
          const d: T.Destination = { ...b, id: `dest-${uuid().slice(0, 8)}`, created_ms: now() };
          destinations.push(d);
          return json(d);
        }
        return json(destinations);
      }
      const d = destinations.find((x) => x.id === seg[1]);
      if (!d) return err("not found", 404);
      if (seg[2] === "test") return d.kind === "rclone" ? json({ ok: false, error: "rclone is not installed" }) : json({ ok: true, free_bytes: 3.2 * 1024 * GB });
      if (method === "DELETE") {
        destinations.splice(destinations.indexOf(d), 1);
        return json({ ok: true });
      }
      Object.assign(d, await body<T.DestinationInput>());
      return json(d);
    }
    case "dialog":
      return json({ path: "D:\\Archive" });
    case "search": {
      const kind = seg[2] as T.SearchKind | undefined;
      if (!kind) return json(searchProviders);
      const sp = searchProviders.find((x) => x.kind === kind);
      if (!sp) return err("unknown provider", 404);
      if (seg[3] === "activate") {
        searchProviders.forEach((x) => (x.is_active = x === sp));
        return json({ ok: true });
      }
      if (seg[3] === "test") return json({ ok: true, latency_ms: 310 });
      const b = await body<T.SearchProviderInput>();
      if (b.api_key) sp.has_key = true;
      if (b.base_url !== undefined) sp.base_url = b.base_url;
      return json({ ok: true });
    }
    case "volumes": {
      if (method === "PUT") {
        const b = await body<Partial<T.VolumeChoice>>();
        const rootOf = (r: string) => {
          const u = r.trim().toUpperCase().replace(/\//g, "\\");
          return /^[A-Z]:$/.test(u) ? `${u}\\` : u;
        };
        const watched = [...new Set((b.watched ?? []).map(rootOf))];
        const selected = rootOf(b.selected ?? "");
        if (!watched.length) return err("at least one drive must stay watched");
        if (!watched.includes(selected)) return err(`${selected} is not among the watched drives`);
        const unknown = watched.find((r) => !drives.some((d) => d.root === r));
        if (unknown) return err(`${unknown} is not a drive path like D:\\`);
        const added = watched.filter((r) => !drives.find((d) => d.root === r)!.watched);
        const gone = added.find((r) => !drives.find((d) => d.root === r)!.connected);
        if (gone) return err(`${gone} is not connected`);
        await sleep(added.length || watched.length < watchedDrives().length ? 600 : 150);
        for (const d of drives) Object.assign(d, { watched: watched.includes(d.root), selected: d.root === selected });
        status.volumes = watchedDrives();
        if (added.length && status.index.state === "idle") startRefresh("fast");
        dirty = true;
      }
      return json(drives);
    }
    case "history":
      return json(history);
    case "settings":
      if (method === "PUT") {
        Object.assign(settings, await body<Partial<T.Settings>>(), { index_size_bytes: settings.index_size_bytes });
        if (settings.index_speed === "fast" && status.index.state === "refreshing" && status.index.mode === "background")
          startRefresh("fast", status.index.progress?.fraction ?? 0);
      }
      return json(settings);
    case "presence":
      return json({ ok: true });
    case "archive":
      return json([]);
    case "builtin":
      if (seg[1] === "install") return json(installBuiltin((await body<{ model_id: string }>()).model_id));
      if (method === "DELETE") {
        const freed = builtin.runtime.size + (builtin.model?.size ?? 0);
        Object.assign(builtin, { installed: false, running: false, model: null, provider_id: null });
        builtin.runtime.installed = false;
        for (const c of builtin.candidates) c.installed = false;
        return json({ ok: true, freed_bytes: freed });
      }
      return json(builtin);
  }
  return err(`no mock for ${method} ${path}`, 404);
}

const builtin: T.BuiltinView = {
  installed: false,
  running: false,
  runtime: { version: "b11503", size: 19_483_533, installed: false, license: "MIT", license_url: "https://github.com/ggml-org/llama.cpp/blob/b11503/LICENSE" },
  model: null,
  candidates: [
    { id: "lfm2.5-1.2b-instruct", name: "LFM2.5 1.2B Instruct", size: 730_895_168, license: "LFM Open License v1.0", license_url: "https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct-GGUF", installed: false, recommended: false },
    { id: "qwen3-4b-instruct-2507", name: "Qwen3 4B Instruct 2507", size: 2_497_281_120, license: "Apache-2.0", license_url: "https://www.apache.org/licenses/LICENSE-2.0", installed: false, recommended: true },
  ],
  free_bytes: 62 * GB,
  recommended: "qwen3-4b-instruct-2507",
  install_job: null,
  provider_id: null,
  threads: 10,
  context: 16384,
};

function installBuiltin(modelId: string): T.Job {
  const pick = builtin.candidates.find((c) => c.id === modelId)!;
  const total = builtin.runtime.size + pick.size;
  const job: T.Job = { id: uuid(), kind: "builtin_install", proposal_id: null, status: "running", progress: { done_bytes: 0, total_bytes: total, done_items: 0, total_items: 2, current: "llama.cpp b11503" }, freed_bytes: 0, error: null, created_ms: now(), finished_ms: null };
  builtin.install_job = job;
  emitJob(job);
  void (async () => {
    for (let s = 1; s <= 20; s++) {
      await sleep(300);
      job.progress.done_bytes = (total * s) / 20;
      job.progress.current = s < 2 ? "llama.cpp b11503" : pick.name;
      emitJob(job);
    }
    Object.assign(job, { status: "done", finished_ms: now(), progress: { ...job.progress, done_items: 2, current: null } });
    builtin.runtime.installed = true;
    pick.installed = true;
    Object.assign(builtin, { installed: true, model: pick, provider_id: "builtin" });
    emitJob(job);
  })();
  return job;
}

// Status stream needs raw comment frames, so it is served outside sseResponse.
function statusStream(): Response {
  let push: ((f: string) => void) | null = null;
  const stream = new ReadableStream({
    start(controller) {
      push = (f) => {
        try {
          controller.enqueue(enc.encode(f));
        } catch {
          // stream already closed
        }
      };
      subscribers.add(push);
      push(`data: ${JSON.stringify({ type: "status", ...status })}\n\n`);
    },
    cancel() {
      if (push) subscribers.delete(push);
    },
  });
  return new Response(stream, { headers: { "content-type": "text/event-stream", "cache-control": "no-cache" } });
}

Bun.serve({
  port: PORT,
  hostname: "127.0.0.1",
  idleTimeout: 0,
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === "/api/status/stream") {
      const auth = req.headers.get("authorization");
      if (auth !== `Bearer ${TOKEN}` && url.searchParams.get("token") !== TOKEN) return err("unauthorized", 401);
      return statusStream();
    }
    try {
      return await route(req);
    } catch (e) {
      return err(e instanceof Error ? e.message : String(e), 500);
    }
  },
});
console.log(`hoardini mock on http://127.0.0.1:${PORT} (token "${TOKEN}")`);
