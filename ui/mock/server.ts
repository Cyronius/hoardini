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
  const mk = (id: number, path: string, alloc: number, p: Partial<T.ProposalItem>): T.ProposalItem => ({ id, path, kind: "file", alloc, verdict: "redownload", cap: null, reason: null, block_reason: null, selected: true, status: "pending", detail: null, small: false, memory_note: null, ...p });
  const items: T.ProposalItem[] = [
    mk(1, "C:\\Users\\josha\\.lmstudio\\models\\llama-3.3-70b-Q4_K_M.gguf", 42.5 * GB, { reason: "Public model on Hugging Face; last opened 74 days ago." }),
    mk(2, "C:\\Users\\josha\\.lmstudio\\models\\mistral-small-24b-Q6_K.gguf", 9.9 * GB, { verdict: "review", cap: "review", reason: "Model said safe, but it was opened 12 days ago." }),
    mk(3, "C:\\Users\\josha\\.lmstudio\\models\\qwen3.8-27b-Q8_0.gguf", 27.1 * GB, { verdict: "keep", selected: false, reason: "You said to keep this one (Sep 12)." }),
    mk(4, "C:\\Users\\josha\\.lmstudio\\models\\gemma-4-12b-Q8_0.gguf", 12.4 * GB, { verdict: "redownload", selected: false, block_reason: "Open in LM Studio.exe (pid 8124)", status: "blocked" }),
    mk(5, "C:\\Users\\josha\\.lmstudio\\models\\tiny-test.gguf", 3 * MB, { selected: false, small: true, reason: "Scratch test model.", memory_note: "C:\\Users\\josha\\.lmstudio is disposable" }),
    ...Array.from({ length: 10 }, (_, n) => mk(6 + n, `C:\\Users\\josha\\.lmstudio\\cache\\blob-${n}.bin`, (180 - n * 12) * MB, { reason: "Cache blob; rebuilt on demand." })),
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
    parent_id: null,
    follow_up_id: null,
  };
  proposals.set(p.id, p);
  return p;
}

// Set through POST /api/mock {"next_job": ...}; shapes the next approval for checking the mascot's moves.
type NextJob = { kind?: T.ProposalKind; outcome?: "failed" | "cancelled"; ms?: number; cloud?: boolean };
let nextJob: NextJob | null = null;

function followUp(p: T.Proposal): void {
  const items = p.items.filter((i) => i.status !== "done").map((i) => ({ ...i, status: i.block_reason ? ("blocked" as const) : ("pending" as const), detail: null }));
  if (!items.length) return;
  const f: T.Proposal = { ...p, id: uuid(), status: "pending", job_id: null, created_ms: now(), items, total_alloc: items.filter((i) => i.selected).reduce((a, i) => a + i.alloc, 0), parent_id: p.id, follow_up_id: null };
  proposals.set(f.id, f);
  thread.proposals[f.id] = f;
  p.follow_up_id = f.id;
}

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
      if (plan.outcome === "failed") for (const it of chosen) it.status = "failed";
      if (p.kind !== "dedupe") followUp(p);
      emitJob(job);
      dirty = true;
      return;
    }
    job.status = "done";
    job.freed_bytes = total;
    job.finished_ms = now();
    p.status = "done";
    if (p.kind !== "dedupe") followUp(p);
    status.vanished_total_bytes += total;
    selectedDrive().free_bytes += total;
    emitJob(job);
    dirty = true;
  })();
  return job;
}

// ---- Folder tree (GET /api/index/children) ----
// Profile paths show as "~", as the real server sends them.
type TreeNode = { path: string; name: string; kind: "dir" | "file"; size: number; files: number; mtime_ms: number; kids?: TreeNode[]; small?: { count: number; bytes: number } };
const DAY = 86_400_000;
const hash = (s: string) => [...s.toLowerCase()].reduce((h, c) => (Math.imul(h, 31) + c.charCodeAt(0)) >>> 0, 7);
const join = (p: string, n: string) => (p.endsWith("\\") ? `${p}${n}` : `${p}\\${n}`);

type Spec = [name: string, gb: number, kids?: Spec[]];
function build(parent: string, [name, gb, kids]: Spec): TreeNode {
  const path = parent === "" ? `${name}\\` : parent === "C:\\Users" && name === "josha" ? "~" : join(parent, name);
  const isFile = /\.(gguf|bin|iso|safetensors|vhdx|mp4|sys|exe|zip)$/i.test(name) && !kids;
  const node: TreeNode = { path, name, kind: isFile ? "file" : "dir", size: Math.round(gb * GB), files: isFile ? 1 : 0, mtime_ms: now() - (hash(path) % 300) * DAY };
  if (kids) {
    node.kids = kids.map((k) => build(path, k));
    const rest = node.size - node.kids.reduce((a, k) => a + k.size, 0);
    if (rest > 0) node.small = { count: 40 + (hash(path) % 900), bytes: rest };
    node.files = node.kids.reduce((a, k) => a + k.files, 0) + (node.small?.count ?? 0);
  } else if (!isFile) node.files = 20 + (hash(path) % 4000);
  return node;
}

const smallRepos: Spec[] = Array.from({ length: 58 }, (_, i) => [`side-project-${String(i + 1).padStart(2, "0")}`, 0.5 - i * 0.007]);
const tree = build("", [
  "C:",
  1844,
  [
    ["Users", 652, [["josha", 650, [[".lmstudio", 96.1, [["models", 95.4, [["llama-3.3-70b-Q4_K_M.gguf", 42.5], ["qwen3.8-27b-Q8_0.gguf", 27.1], ["gemma-4-12b-Q8_0.gguf", 12.4], ["mistral-small-24b-Q6_K.gguf", 9.9]]], ["cache", 0.6]]], ["AppData", 120, [["Local", 110, [["Docker", 40], ["Temp", 12, [["claude", 9.1]]], ["Microsoft", 21]]], ["Roaming", 9]]], [".cache", 48, [["huggingface", 31], ["pip", 6.2], ["uv", 4.6]]], ["Downloads", 31], ["Documents", 22]]]]],
    ["code", 312, [["covenant-agent", 161, [["target", 98], ["data", 41], ["models", 17], ["node_modules", 3.2], ["src", 0.4]]], ["hoardini", 58, [["target", 52], ["ui", 4.1]]], ["llama.cpp", 31], ["based", 22], ["lm-admin", 14], ...smallRepos]],
    ["Games", 160],
    ["Program Files", 88],
    ["Windows", 41],
    ["ProgramData", 19],
    ["$WINDOWS.~BT", 9.2],
    ["NVIDIA", 3.1],
    ["Intel", 1.2],
    ["PerfLogs", 0.1],
  ],
]);

const NAMES = ["cache", "data", "build", "models", "packages", "logs", "assets", "lib", "bin", "backup", "tmp", "src"];
// Folders the tree above leaves open get made-up contents, so ▸ always goes deeper.
function grow(n: TreeNode): void {
  if (n.kids || n.kind === "file") return;
  const h = hash(n.path);
  if (n.size < 64 * MB) {
    n.small = { count: n.files, bytes: n.size };
    n.kids = [];
    return;
  }
  const k = 3 + (h % 5);
  let left = n.size * 0.9;
  n.kids = Array.from({ length: k }, (_, i) => {
    const size = Math.round(i === k - 1 ? left : left * (0.45 + ((h >> i) % 20) / 100));
    left -= size;
    const name = i === 0 && n.size < 4 * GB ? `${NAMES[h % NAMES.length]}-${i}.bin` : `${NAMES[(h + i) % NAMES.length]}${i > 3 ? `-${i}` : ""}`;
    const isFile = name.endsWith(".bin");
    return { path: join(n.path, name), name, kind: isFile ? "file" : "dir", size, files: isFile ? 1 : 10 + ((h >> i) % 3000), mtime_ms: now() - ((h >> i) % 200) * DAY };
  });
  n.small = { count: 30 + (h % 500), bytes: Math.round(n.size * 0.1) };
}

const norm = (p: string) => p.trim().replace(/\//g, "\\").replace(/^C:\\Users\\josha(?=\\|$)/i, "~").replace(/\\+$/, "").toLowerCase();

function findNode(path: string): TreeNode | null {
  const want = norm(path);
  const walk = (n: TreeNode): TreeNode | null => {
    if (norm(n.path) === want) return n;
    grow(n);
    for (const k of n.kids ?? []) {
      const p = norm(k.path);
      if (want === p || want.startsWith(`${p}\\`)) return walk(k);
    }
    return null;
  };
  const profile = tree.kids!.find((k) => k.name === "Users")!.kids!.find((k) => k.path === "~")!;
  return walk(want === "~" || want.startsWith("~\\") ? profile : tree);
}

function childrenOf(path: string, limit: number) {
  const n = findNode(path);
  if (!n) return null;
  grow(n);
  const kids = [...(n.kids ?? [])].sort((a, b) => b.size - a.size || a.name.localeCompare(b.name));
  const rows = kids.slice(0, limit).map((k) => ({ path: k.path, name: k.name, kind: k.kind, size: k.size, files: k.files, mtime_ms: k.mtime_ms, denied: false, has_children: k.kind === "dir" }));
  const rest = kids.slice(limit);
  return { path: n.path, size: n.size, files: n.files, children: rows, small: n.small ?? { count: 0, bytes: 0 }, more: { count: rest.length, bytes: rest.reduce((a, k) => a + k.size, 0) } };
}

const revealed: string[] = [];

// ---- AG-UI run ----
const enc = new TextEncoder();
type Sse = { send: (e: Record<string, unknown>) => void; closed: () => boolean };
type Stored = { id: string } & Record<string, unknown>;

async function say(s: Sse, text: string) {
  const messageId = uuid();
  s.send({ type: "TEXT_MESSAGE_START", messageId, role: "assistant" });
  for (const chunk of text.match(/[\s\S]{1,18}(\s|$)/g) ?? [text]) {
    if (s.closed()) return messageId;
    s.send({ type: "TEXT_MESSAGE_CONTENT", messageId, delta: chunk });
    await sleep(35);
  }
  s.send({ type: "TEXT_MESSAGE_END", messageId });
  thread.messages.push({ id: messageId, role: "assistant", content: text });
  return messageId;
}

// Like the server: the call joins the text message it follows, and its card data comes right after the result.
async function readTool(s: Sse, name: string, args: unknown, result: string, ms = 600, data?: unknown, parent?: string) {
  const toolCallId = uuid();
  const parentMessageId = parent ?? uuid();
  s.send({ type: "TOOL_CALL_START", toolCallId, toolCallName: name, parentMessageId });
  s.send({ type: "TOOL_CALL_ARGS", toolCallId, delta: JSON.stringify(args) });
  s.send({ type: "TOOL_CALL_END", toolCallId });
  await sleep(ms);
  const resultId = uuid();
  s.send({ type: "TOOL_CALL_RESULT", messageId: resultId, toolCallId, content: result, role: "tool" });
  if (data) s.send({ type: "CUSTOM", name: "tool_data", value: { tool_call_id: toolCallId, data } });
  const call = { id: toolCallId, type: "function", function: { name, arguments: JSON.stringify(args) } };
  const host = parent ? (thread.messages.find((m) => m.id === parent) as Stored | undefined) : undefined;
  if (host) host.toolCalls = [...((host.toolCalls as unknown[]) ?? []), call];
  else thread.messages.push({ id: parentMessageId, role: "assistant", content: "", toolCalls: [call] });
  thread.messages.push({ id: resultId, role: "tool", content: result, toolCallId, ...(data ? { data } : {}) });
  return toolCallId;
}

const folderData = (under: string | null, depth = 1, limit = 15) => {
  const n = findNode(under ?? "C:\\");
  grow(n!);
  const kids = [...(n!.kids ?? [])].filter((k) => k.kind === "dir").sort((a, b) => b.size - a.size);
  const rest = kids.slice(limit);
  return {
    v: 1,
    kind: "folders",
    under,
    depth,
    as_of_ms: status.index.last_refresh_ms,
    items: kids.slice(0, limit).map((k) => ({ path: k.path, size: k.size, files: k.files, denied: k.name === "Windows" })),
    more: { count: rest.length, bytes: rest.reduce((a, k) => a + k.size, 0) },
  };
};
const folderText = (d: ReturnType<typeof folderData>) =>
  `Largest folders under ${d.under ?? "the indexed volumes"} (depth ${d.depth}; index as of 3 days ago):\n${d.items.map((i) => `- ${i.path} | ${(i.size / GB).toFixed(1)} GB | ${i.files} files`).join("\n")}`;

function filesData() {
  const files = [
    ["~\\.lmstudio\\models\\llama-3.3-70b-Q4_K_M.gguf", 42.5, 74],
    ["~\\.lmstudio\\models\\qwen3.8-27b-Q8_0.gguf", 27.1, 160],
    ["~\\AppData\\Local\\Docker\\wsl\\disk\\docker_data.vhdx", 21.3, 0],
    ["C:\\pagefile.sys", 16, 0],
    ["C:\\hiberfil.sys", 12.7, 0],
    ["~\\.lmstudio\\models\\gemma-4-12b-Q8_0.gguf", 12.4, 150],
    ["~\\.lmstudio\\models\\mistral-small-24b-Q6_K.gguf", 9.9, 180],
    ["~\\Downloads\\Win11_25H2_English_x64.iso", 7, 200],
    ["C:\\code\\covenant-agent\\models\\encoder.safetensors", 6.5, 33],
    ["~\\Downloads\\ubuntu-24.04.3-desktop-amd64.iso", 5.9, 150],
    ["~\\Documents\\Videos\\wedding-2019.mp4", 4.2, 900],
    ["C:\\code\\hoardini\\target\\debug\\incremental\\query-cache.bin", 4.2, 1],
  ] as const;
  return {
    v: 1,
    kind: "files",
    filters: { under: null, min_size: null, ext: null, not_used_for_days: null },
    as_of_ms: status.index.last_refresh_ms,
    items: files.map(([path, gb, days]) => ({ path, size: Math.round(gb * GB), modified_ms: now() - days * DAY, accessed_ms: now() - Math.min(days, 30) * DAY })),
    more: { count: 3184, bytes: Math.round(212 * GB) },
  };
}

function changesData() {
  const from = new Date();
  from.setHours(14, 14, 0, 0);
  if (from.getTime() > now()) from.setTime(now() - 3 * 3_600_000);
  const c = (kind: "dir" | "file", path: string, before: number | null, after: number | null) => ({
    kind,
    path,
    before: before == null ? null : Math.round(before * GB),
    after: after == null ? null : Math.round(after * GB),
    delta: Math.round(((after ?? 0) - (before ?? 0)) * GB),
  });
  return {
    v: 1,
    kind: "changes",
    volume: "C:\\",
    from: { id: 41, ms: from.getTime() },
    to: { id: 42, ms: now() - 5 * 60_000 },
    free_delta: Math.round(127 * GB),
    indexed_delta: Math.round(-129.5 * GB),
    free_before: Math.max(0, selectedDrive().free_bytes - Math.round(127 * GB)),
    free_after: selectedDrive().free_bytes,
    grew: [c("dir", "C:\\code\\covenant-agent\\data", 36.9, 41), c("dir", "~\\AppData\\Local\\Docker", 38.2, 40)],
    new: [c("file", "~\\Downloads\\proxmox-ve_9.0-1.iso", null, 1.4)],
    shrank: [c("dir", "C:\\", 1973.5, 1844), c("dir", "~\\.cache", 68, 48), c("dir", "~\\.lmstudio\\models", 107.4, 95.4)],
    removed: [c("dir", "~\\AppData\\Local\\Temp\\claude\\scratch", 92, null), c("dir", "~\\.cache\\huggingface\\hub\\models--meta-llama--Llama-3.1-8B", 15.1, null), c("file", "~\\.lmstudio\\models\\deepseek-r1-32b-Q4_K_M.gguf", 12, null)],
    omitted: { grew: 0, new: 2, shrank: 1, removed: 0 },
    range_ms: [from.getTime(), now() - 5 * 60_000],
  };
}

async function suggest(s: Sse, parent: string, items: { label: string; kind: "ask" | "clean_up"; path?: string }[]) {
  const kept = items.filter((p) => !p.path || findNode(p.path));
  await readTool(s, "suggest_next", { items }, `Shown under your answer: ${kept.map((p) => JSON.stringify(p.label)).join(", ")}.`, 150, kept.length ? { v: 1, kind: "pills", items: kept } : undefined, parent);
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
      } else if (/hotspot|where .*space|taking up/i.test(text)) {
        const folders = folderData(null);
        await readTool(s, "top_dirs", {}, folderText(folders), 500, folders);
        await readTool(s, "changes", { since: "previous" }, "C:\\: refresh #41 -> #42. Free space +127.0 GB.\nrange_ms: 0..0 C:\\", 500, changesData());
        const id = await say(s, "Most of the change is the Claude scratch folder in Temp going away. `C:\\code` still holds 312 GB, most of it in covenant-agent.");
        await suggest(s, id, [
          { label: "What's in covenant-agent?", kind: "ask", path: "C:\\code\\covenant-agent" },
          { label: "Clean up C:\\code", kind: "clean_up", path: "C:\\code" },
          { label: "Explain a gone folder", kind: "ask", path: "C:\\nope" },
        ]);
      } else if (/biggest files|largest files/i.test(text)) {
        const files = filesData();
        await readTool(s, "top_files", { limit: 12 }, files.items.map((f) => `- ${f.path}`).join("\n"), 500, files);
        const id = await say(s, "Model files lead, then the Docker disk and the page file.");
        await suggest(s, id, [{ label: "Which models haven't I used lately?", kind: "ask" }]);
      } else if (/over time|free space changed/i.test(text)) {
        await readTool(s, "changes", { since: "30d" }, "C:\\: refresh #28 -> #42.\nrange_ms: 0..0 C:\\", 500, changesData());
        await say(s, "Free space has trended down, with a jump today when the Temp scratch folder went.");
      } else if (/^what's inside (.+)\?$|^what can i clean up in (.+)\?$/i.test(text)) {
        const m = /^what's inside (.+)\?$|^what can i clean up in (.+)\?$/i.exec(text)!;
        const path = (m[1] ?? m[2])!;
        const folders = findNode(path) ? folderData(path) : null;
        if (folders) await readTool(s, "top_dirs", { under: path }, folderText(folders), 400, folders);
        const id = await say(s, folders ? `Here is what is inside \`${path}\`.` : `\`${path}\` is not in the index.`);
        if (folders?.items[0]) await suggest(s, id, [{ label: `Clean up ${folders.items[0].path}`, kind: "clean_up", path: folders.items[0].path }]);
      } else if (/^what is (.+) and is it safe to remove\?$/i.test(text)) {
        const path = /^what is (.+) and is it safe to remove\?$/i.exec(text)![1]!;
        await readTool(s, "inspect", { path }, `${path}: build output and caches; nothing the user made.`, 400);
        const id = await say(s, `\`${path}\` is mostly build output that comes back on the next build, so it is safe to remove.`);
        await suggest(s, id, [{ label: `Clean up ${path}`, kind: "clean_up", path }]);
      } else {
        const id = await say(s, "Let me look at what is in `~\\.lmstudio`.\n\n");
        const lm = folderData("~\\.lmstudio");
        await readTool(s, "top_dirs", { under: "~\\.lmstudio", depth: 2, limit: 10 }, "~\\.lmstudio 96.1 GB\n  models 95.4 GB (9 files)\n  cache 0.6 GB", 700, lm, id);
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
      if (seg[1] === "revealed") return json(revealed);
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
    case "reveal": {
      const { path: p } = await body<{ path?: string }>();
      if (!p?.trim()) return err("path is required");
      if (!findNode(p)) return err(`${p} not found`, 404);
      revealed.push(p);
      console.log(`reveal ${p}`);
      return json({ ok: true });
    }
    case "index":
      if (seg[1] === "summary") return json(summary);
      if (seg[1] === "children") {
        const p = url.searchParams.get("path");
        if (!p?.trim()) return err("path is required");
        const page = childrenOf(p, Math.min(1000, Math.max(1, Number(url.searchParams.get("limit") ?? 50))));
        if (!page) return err(`${p} in the index not found`, 404);
        if (findNode(p)!.kind === "file") return err(`${p} is not a folder`);
        return json(page);
      }
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
