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
    mode: "fast",
    progress: { phase: "walk", dirs: 120_000, files: 2_300_000, bytes: 700 * GB, fraction: 0.4 },
    last_refresh_ms: process.env.MOCK_NO_INDEX ? null : now() - 3 * 86_400_000,
    generation: 12,
  },
  jobs: [],
  vanished_total_bytes: Math.round(52.4 * GB),
  active_provider: { id: "prov-lmstudio", name: "LM Studio", model: "qwen3.8-27b" },
  running_runs: 0,
  review: null,
  review_building: false,
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
      rebuildIfUntouched();
    }
  }, 700);
  dirty = true;
}
if (process.env.MOCK_NO_INDEX) startRefresh("fast", 0);
else if (!process.env.MOCK_IDLE) startRefresh("fast", 0.4);
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
  { id: 4, scope: "path", key: "~\\Pictures", kind: "verdict", text: "Your photos and videos: 93% of its bytes are images or video.", source: "mix", confidence: null, created_ms: now() - 3 * 86_400_000, updated_ms: now() - 3 * 86_400_000, review_after_ms: null, pinned: false, last_used_ms: now() - 86_400_000, forgotten_ms: null, detail: { category: "photos", label: "your photos and videos", verdict: "keep", actions: ["archive"], size_at: 80 * GB, assessed_ms: now() - 3 * 86_400_000, by: "mix" } },
  { id: 5, scope: "glob", key: "**\\*.q4nx", kind: "fact", text: ".q4nx: FastFlowLM NPU model weights.", source: "web:https://huggingface.co/FastFlowLM", confidence: null, created_ms: now() - 2 * 86_400_000, updated_ms: now() - 2 * 86_400_000, review_after_ms: null, pinned: false, last_used_ms: now() - 86_400_000, forgotten_ms: null, detail: { category: "model-weights", app: "FastFlowLM", use: "NPU model weights" } },
  { id: 3, scope: "global", key: "", kind: "preference", text: "Prefer archiving to the NAS over deleting when a file is bigger than 10 GB.", source: "chat 2026-10-01", confidence: null, created_ms: now() - 7 * 86_400_000, updated_ms: now() - 7 * 86_400_000, review_after_ms: null, pinned: false, last_used_ms: now() - 7 * 86_400_000, forgotten_ms: now() - 3 * 86_400_000 },
];
let memLimit = 200;
let memSeq = 10;

const providers: T.ProviderView[] = [
  { id: "prov-lmstudio", name: "LM Studio", base_url: "http://127.0.0.1:1234/v1", model: "qwen3.8-27b", api_key: null, context_window: 32768, timeout_s: 180, max_tool_steps: 20, small: false, params: { temperature: 0.2 }, headers: {}, extra_body: {}, is_active: true, has_key: false, caps: { tools: true, streaming: true, streamed_tool_calls: true, tool_choice: true } },
];
const presets: T.Preset[] = [
  { id: "cerebras", name: "Cerebras", base_url: "https://api.cerebras.ai/v1", needs_key: true, default_model: "gpt-oss-120b", key_url: "https://cloud.cerebras.ai/platform/" },
  { id: "gemini", name: "Google Gemini", base_url: "https://generativelanguage.googleapis.com/v1beta/openai/", needs_key: true, default_model: "gemini-3.5-flash-lite", key_url: "https://aistudio.google.com/apikey" },
  { id: "openrouter", name: "OpenRouter", base_url: "https://openrouter.ai/api/v1", needs_key: true, default_model: "google/gemma-4-31b-it:free", key_url: "https://openrouter.ai/settings/keys" },
  { id: "mistral", name: "Mistral", base_url: "https://api.mistral.ai/v1", needs_key: true, default_model: "mistral-small-2603", key_url: "https://console.mistral.ai/api-keys" },
  { id: "zai", name: "Z.ai", base_url: "https://api.z.ai/api/paas/v4", needs_key: true, default_model: "glm-4.7-flash", key_url: "https://z.ai/manage-apikey/apikey-list" },
  { id: "ollama-cloud", name: "Ollama Cloud", base_url: "https://ollama.com/v1", needs_key: true, default_model: "gemma4:31b", key_url: "https://ollama.com/settings/keys" },
  { id: "cloudflare", name: "Cloudflare Workers AI", base_url: "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1", needs_key: true, default_model: "@cf/zai-org/glm-4.7-flash", key_url: "https://dash.cloudflare.com/profile/api-tokens" },
  { id: "lmstudio", name: "LM Studio", base_url: "http://127.0.0.1:1234/v1", needs_key: false, default_model: null, key_url: null },
  { id: "ollama", name: "Ollama", base_url: "http://127.0.0.1:11434/v1", needs_key: false, default_model: null, key_url: null },
  { id: "openai", name: "OpenAI", base_url: "https://api.openai.com/v1", needs_key: true, default_model: null, key_url: null },
];
let setupDone = true;
const secrets = new Set<string>();
const serverModels = ["qwen3.8-27b", "llama-3.3-70b-instruct", "mistral-small-24b", "gemma-4-12b"];

const destinations: T.Destination[] = [{ id: "dest-nas", name: "NAS", kind: "smb", config: { path: "\\\\nas\\backups\\hoardini", username: "josh" }, created_ms: now() - 20 * 86_400_000 }];

const searchProviders: T.SearchProviderView[] = (["tavily", "brave", "exa", "serper", "searxng"] as T.SearchKind[]).map((kind) => ({ kind, is_active: kind === "tavily", has_key: kind === "tavily", base_url: null }));

const settings: T.Settings = {
  builtin_idle_minutes: 10,
  index_size_bytes: 184 * MB,
  online_checks: { asked: true, services: { huggingface: true, winget: true } },
};

const SESSION_GAP_MS = 6 * 3_600_000;
const proposals = new Map<string, T.Proposal>();

// Set through POST /api/mock {"next_job": ...}; shapes the next approval for checking the mascot's moves.
type NextJob = { kind?: T.ProposalKind; outcome?: "failed" | "cancelled"; ms?: number; cloud?: boolean; startDelayMs?: number; rejectApprove?: boolean };
let nextJob: NextJob | null = null;

function followUp(p: T.Proposal): void {
  const items = p.items.filter((i) => i.status !== "done").map((i) => ({ ...i, status: i.block_reason ? ("blocked" as const) : ("pending" as const), detail: null }));
  if (!items.length) return;
  const f: T.Proposal = { ...p, id: uuid(), status: "pending", job_id: null, created_ms: now(), items, total_alloc: items.filter((i) => i.selected).reduce((a, i) => a + i.alloc, 0), parent_id: p.id, follow_up_id: null, touched: false };
  proposals.set(f.id, f);
  p.follow_up_id = f.id;
}

function commonFolder(paths: string[]): string {
  const parts = paths.map((x) => x.split("\\").slice(0, -1));
  const out: string[] = [];
  for (let i = 0; parts.every((q) => q[i] != null && q[i]!.toLowerCase() === parts[0]![i]!.toLowerCase()); i++) out.push(parts[0]![i]!);
  return out.join("\\") || paths[0] || "~";
}

// Like MEM-LEARN-JOB: done items become `cleaned`, items left unticked `kept`, on one row the job made.
function learnFrom(p: T.Proposal): void {
  if (p.kind === "dedupe") return;
  const done = p.items.filter((i) => i.status === "done");
  const kept = p.items.filter((i) => !i.selected && i.status === "pending");
  if (!done.length && !kept.length) return;
  const ms = now();
  const tilde = (path: string) => path.replace(/^C:\\Users\\josha/i, "~");
  const where = commonFolder(p.items.map((i) => tilde(i.path)));
  const n = (k: number) => `${k} item${k === 1 ? "" : "s"}`;
  const m: T.Memory = {
    id: ++memSeq,
    scope: "path",
    key: where,
    kind: "verdict",
    text: done.length ? `You cleaned ${n(done.length)} here.` : `You kept ${n(kept.length)} here.`,
    source: `job:${p.id}`,
    confidence: null,
    created_ms: ms,
    updated_ms: ms,
    review_after_ms: null,
    pinned: false,
    last_used_ms: ms,
    forgotten_ms: null,
    detail: {
      category: "unknown",
      by: "job",
      cleaned: done.map((i) => ({ ms, bytes: i.alloc, path: tilde(i.path), rule: tilde(i.path), job: p.id, kind: p.kind })),
      kept: kept.map((i) => ({ ms, path: tilde(i.path), size: i.alloc, job: p.id })),
    },
  };
  memories.push(m);
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
      if (it.op?.op === "leftovers") it.detail = `2,412 files deleted (${(it.alloc / GB).toFixed(1)} GB); left: 3 in use, 41 newer than a day, 0 could not be deleted`;
      job.progress.done_items = n + 1;
    }
    if (plan?.outcome) {
      Object.assign(job, { status: plan.outcome, error: plan.outcome === "failed" ? "Access is denied" : null, finished_ms: now() });
      p.status = "failed";
      if (plan.outcome === "failed") for (const it of chosen) it.status = "failed";
      if (p.kind !== "dedupe" && p.kind !== "system_cleanup") followUp(p);
      learnFrom(p);
      emitJob(job);
      dirty = true;
      return;
    }
    job.status = "done";
    job.freed_bytes = total;
    job.finished_ms = now();
    p.status = "done";
    if (p.kind !== "dedupe" && p.kind !== "system_cleanup") followUp(p);
    learnFrom(p);
    status.vanished_total_bytes += total;
    selectedDrive().free_bytes += total;
    emitJob(job);
    dirty = true;
  })();
  return job;
}

type TreeNode = { path: string; name: string; kind: "dir" | "file"; size: number; files: number; mtime_ms: number; kids?: TreeNode[]; small?: { count: number; bytes: number }; parent?: TreeNode; grown?: boolean };
const DAY = 86_400_000;
const hash = (s: string) => [...s.toLowerCase()].reduce((h, c) => (Math.imul(h, 31) + c.charCodeAt(0)) >>> 0, 7);
const join = (p: string, n: string) => (p.endsWith("\\") ? `${p}${n}` : `${p}\\${n}`);
const FILE_RE = /\.(gguf|q4nx|bin|iso|safetensors|vhdx|mp4|sys|exe|zip|dll|pak|cab)$/i;

type Spec = [name: string, gb: number, kids?: Spec[]];
function build(parent: string, [name, gb, kids]: Spec, up?: TreeNode): TreeNode {
  const path = parent === "" ? `${name}\\` : parent === "C:\\Users" && name === "josha" ? "~" : join(parent, name);
  const isFile = FILE_RE.test(name) && !kids;
  const node: TreeNode = { path, name, kind: isFile ? "file" : "dir", size: Math.round(gb * GB), files: isFile ? 1 : 0, mtime_ms: now() - (hash(path) % 300) * DAY, parent: up };
  if (kids) {
    node.kids = kids.map((k) => build(path, k, node));
    const rest = node.size - node.kids.reduce((a, k) => a + k.size, 0);
    if (rest > 0) node.small = { count: 40 + (hash(path) % 900), bytes: rest };
    node.files = node.kids.reduce((a, k) => a + k.files, 0) + (node.small?.count ?? 0);
  } else if (!isFile) node.files = 20 + (hash(path) % 4000);
  return node;
}

const smallRepos: Spec[] = Array.from({ length: 58 }, (_, i) => [`side-project-${String(i + 1).padStart(2, "0")}`, 0.5 - i * 0.007]);
// Shaped like the maintainer's C: drive: a 725 GB profile, 365 GB of it in AppData\Local.
const tree = build("", [
  "C:",
  1810,
  [
    [
      "Users",
      727,
      [
        [
          "josha",
          725,
          [
            [
              "AppData",
              372,
              [
                [
                  "Local",
                  365,
                  [
                    [
                      "Temp",
                      101,
                      [
                        [
                          "claude",
                          92,
                          [
                            ["c--code-hoardini", 41, [["3f9c2d1e", 23, [["Qwen3.6-35B-A3B-NPU2.q4nx", 21.4]]], ["8a71b0c4", 17.8, [["gemma-4-12b-Q8_0.gguf", 12.4], ["phi-5-mini-NPU2.q4nx", 5.1]]]]],
                            ["c--code-covenant-agent", 33, [["51e2aa90", 33, [["Qwen3.8-27B-NPU2.q4nx", 28.9], ["tokenizer-cache.bin", 3.6]]]]],
                            ["c--code-based", 17.6, [["0d4be7f2", 17.6, [["llama-3.3-8b-NPU2.q4nx", 8.2], ["mistral-small-24b-Q4_K_M.gguf", 9.1]]]]],
                          ],
                        ],
                        ["WinGet", 3.1],
                      ],
                    ],
                    ["Packages", 122, [["CanonicalGroupLimited.Ubuntu24.04LTS_79rhkp1fndgsc", 121.5, [["LocalState", 121.2, [["ext4.vhdx", 121]]]]]]],
                    ["Docker", 82, [["wsl", 81.5, [["disk", 81.5, [["docker_data.vhdx", 78.2]]]]]]],
                    ["Google", 46, [["Chrome", 46, [["User Data", 45.9, [["Default", 45.6, [["Service Worker", 42.4, [["CacheStorage", 42.1]]], ["Cache", 2.4]]]]]]]]],
                    ["Microsoft", 6],
                    ["Programs", 4.9, [["Ollama", 4.8, [["lib", 4.7, [["ollama", 4.6]]]]]]],
                    ["datalab", 2.9, [["datalab", 2.9, [["Cache", 2.9, [["models", 2.9]]]]]]],
                  ],
                ],
                ["Roaming", 7],
              ],
            ],
            [".flm", 104, [["models", 103.6, [["Qwen3.6-35B-A3B-NPU2", 38.1, [["model.q4nx", 37.6]]], ["Qwen3.8-27B-NPU2", 29.2, [["model.q4nx", 28.9]]], ["gemma-4-12b-NPU2", 14.1, [["model.q4nx", 13.8]]], ["Qwen3-Coder-30B-NPU2", 9.4, [["model.q4nx", 9.2]]], ["llama-3.3-8b-NPU2", 8.2, [["model.q4nx", 8]]], ["phi-5-mini-NPU2", 4.5, [["model.q4nx", 4.4]]]]]]],
            [".lmstudio", 96.1, [["models", 95.4, [["llama-3.3-70b-Q4_K_M.gguf", 42.5], ["qwen3.8-27b-Q8_0.gguf", 27.1], ["gemma-4-12b-Q8_0.gguf", 12.4], ["mistral-small-24b-Q6_K.gguf", 9.9]]], ["cache", 0.6]]],
            [".cache", 48, [["huggingface", 31, [["hub", 31, [["models--Qwen--Qwen3-8B-GGUF", 17.2, [["Qwen3-8B-Q8_0.gguf", 8.7], ["Qwen3-8B-Q6_K.gguf", 6.7]]], ["models--black-forest-labs--FLUX.1-dev", 13.6, [["flux1-dev.safetensors", 11.9]]]]]]], ["uv", 13.6], ["pip", 3.2]]],
            ["Downloads", 31, [["Win11_25H2_English_x64.iso", 7], ["ubuntu-24.04.3-desktop-amd64.iso", 5.9], ["proxmox-ve_9.0-1.iso", 1.4], ["OllamaSetup.exe", 1.1], ["balenaEtcher-2.1.7.Setup.exe", 0.16]]],
            ["Documents", 22, [["Videos", 14, [["wedding-2019.mp4", 4.2]]]]],
            [".hipfire", 21],
            ["Pictures", 18, [["2019", 11.2], ["2024", 4.1], ["2026", 2.3]]],
            ["OneDrive", 8, [["Pictures", 5.4, [["Camera Roll 2019", 4.6]]], ["Videos", 2.2, [["drone-2018.mp4", 1.9]]]]],
          ],
        ],
      ],
    ],
    ["code", 312, [["covenant-agent", 161, [["models", 53.6], ["target", 61], ["data", 41], ["node_modules", 3.6], ["src", 0.4]]], ["hoardini", 58, [["target", 52], ["ui", 4.1]]], ["llama.cpp", 31, [["models", 22.4], ["build", 6.2]]], ["based", 22, [["runs", 9.6], [".venv", 4.1]]], ["lm-admin", 14], ...smallRepos]],
    ["Games", 300],
    ["Program Files", 170],
    ["AI", 140, [["gguf", 71.6], ["comfyui-models", 44.2], [".flm-scratch", 21.2, [["Qwen3.6-35B-A3B-NPU2-requant", 21.2, [["model.q4nx", 21]]]]]]],
    [
      "Windows",
      52,
      [
        ["System32", 8.8],
        ["WinSxS", 19.2],
        ["Installer", 6.1],
        ["SoftwareDistribution", 4.6, [["Download", 4.2, [["4b1f9c0e7d2a6f31", 3.4]]]]],
        ["Temp", 1.3, [["{8C1F2B7A-4D0E-4F2B-9C41-6A2E0D5B7F13}", 0.9]]],
        ["LiveKernelReports", 1.1, [["WATCHDOG", 1.1]]],
        ["Logs", 0.5, [["CBS", 0.4, [["CbsPersist_20260912.cab", 0.4]]]]],
      ],
    ],
    [
      "ProgramData",
      44,
      [
        ["Package Cache", 18.4],
        ["Docker", 9.1],
        ["NVIDIA Corporation", 6.3],
        ["chocolatey", 4.8],
        ["Microsoft", 3.2, [["Windows", 1.4, [["WER", 0.7, [["ReportArchive", 0.6]]]]]]],
      ],
    ],
    ["hiberfil.sys", 35.1],
    ["pagefile.sys", 16],
    ["$WINDOWS.~BT", 9.2],
    ["NVIDIA", 3.1],
    ["Intel", 1.2],
  ],
]);

const HINTS: Record<string, string[]> = {
  "c:\\program files": ["dll", "exe", "pak"],
  "~\\appdata\\local\\programs": ["dll", "exe", "pak"],
  "c:\\windows": ["dll", "sys", "cab"],
  "c:\\games": ["pak", "bin"],
  "c:\\ai": ["gguf"],
  "c:\\code": ["bin", "json", "pt"],
  "c:\\code\\covenant-agent\\target": ["rlib", "pdb", "exe"],
  "c:\\code\\hoardini\\target": ["rlib", "pdb", "exe"],
  "c:\\code\\llama.cpp": ["gguf", "bin"],
  "c:\\code\\covenant-agent\\data": ["parquet", "bin"],
  "c:\\code\\covenant-agent\\models": ["safetensors", "gguf"],
  "~\\pictures": ["jpg", "heic", "mp4"],
  "~\\onedrive": ["docx", "pdf", "jpg"],
  "~\\.hipfire": ["bin", "json"],
  "~\\appdata\\local\\microsoft": ["dat", "db", "edb"],
  "~\\appdata\\local\\nvidia": ["bin", "toc"],
  "c:\\programdata": ["dat", "msi", "log"],
};

function hintFor(path: string): string[] {
  let p = path.toLowerCase();
  for (;;) {
    if (HINTS[p]) return HINTS[p]!;
    const i = p.lastIndexOf("\\");
    if (i <= 0) return ["bin"];
    p = p.slice(0, i);
  }
}

const NAMES = ["cache", "data", "build", "models", "packages", "logs", "assets", "lib", "bin", "backup", "tmp", "src"];
// Folders the tree above leaves open get made-up contents, so ▸ always goes deeper.
function grow(n: TreeNode): void {
  if (n.kids || n.kind === "file") return;
  const h = hash(n.path);
  n.grown = true;
  if (n.size < 64 * MB) {
    n.small = { count: n.files, bytes: n.size };
    n.kids = [];
    return;
  }
  const k = 3 + (h % 5);
  const exts = hintFor(n.path);
  let left = n.size * 0.985;
  n.kids = Array.from({ length: k }, (_, i) => {
    const size = Math.round(i === k - 1 ? left : left * (0.45 + ((h >> i) % 20) / 100));
    left -= size;
    const isFile = (i === 0 && n.size < 4 * GB) || (i === 1 && h % 3 === 0);
    const name = isFile ? `${NAMES[(h + i) % NAMES.length]}-${i}.${exts[i % exts.length]}` : `${NAMES[(h + i) % NAMES.length]}${i > 3 ? `-${i}` : ""}`;
    return { path: join(n.path, name), name, kind: isFile ? "file" : "dir", size, files: isFile ? 1 : 10 + ((h >>> i) % 3000), mtime_ms: now() - ((h >>> i) % 200) * DAY, parent: n };
  });
  n.small = { count: 30 + (h % 500), bytes: Math.round(n.size * 0.015) };
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

type ItemSeed = { path: string; gb: number; verdict: T.Verdict; reason: string; kind?: "dir" | "file"; memory_note?: string; copy_note?: string };
type CardSeed = { card: string; title: string; kind: T.ProposalKind; ticked: boolean; summary: string; items: ItemSeed[]; format?: string; ops?: () => T.ProposalItem[] };

const claude = "~\\AppData\\Local\\Temp\\claude";
const cardSeeds = (): CardSeed[] => [
  {
    card: "temp",
    title: "Temp and Windows leftovers",
    kind: "delete",
    ticked: true,
    summary: "Old Claude Code scratch sessions and other temp files nothing has used for a day.",
    items: [
      { path: `${claude}\\c--code-covenant-agent\\51e2aa90`, gb: 33, verdict: "safe", reason: "User temp folder: untouched for 12 days." },
      { path: `${claude}\\c--code-hoardini\\3f9c2d1e`, gb: 23, verdict: "safe", reason: "User temp folder: untouched for 6 days." },
      { path: `${claude}\\c--code-hoardini\\8a71b0c4`, gb: 17.8, verdict: "safe", reason: "User temp folder: untouched for 3 days." },
      { path: `${claude}\\c--code-based\\0d4be7f2`, gb: 17.6, verdict: "safe", reason: "User temp folder: untouched for 21 days." },
      { path: "~\\AppData\\Local\\Temp\\WinGet", gb: 3.1, verdict: "safe", reason: "User temp folder: untouched for 21 days. You cleaned this on 08-14." },
    ],
  },
  {
    card: "caches",
    title: "Caches",
    kind: "delete",
    ticked: true,
    summary: "Browser and package caches; their apps download or rebuild them as needed.",
    items: [
      { path: "~\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Service Worker\\CacheStorage", gb: 42.1, verdict: "safe", reason: "Chrome Service Worker cache: Data websites keep for offline use; Chrome fetches it again." },
      { path: "~\\.cache\\uv", gb: 13.6, verdict: "safe", reason: "uv cache: Python packages uv downloaded; uv fetches them again." },
      { path: "~\\.cache\\pip", gb: 3.2, verdict: "safe", reason: "pip cache: Wheels and HTTP responses pip downloaded." },
      { path: "~\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache", gb: 2.4, verdict: "safe", reason: "Google Chrome cache: Web page cache; Chrome rebuilds it as you browse." },
    ],
  },
  {
    card: "build",
    title: "Build output",
    kind: "delete",
    ticked: true,
    summary: "Build output and dependency folders in C:\\code; each project rebuilds them.",
    items: [
      { path: "C:\\code\\covenant-agent\\target", gb: 61, verdict: "safe", reason: "Rust target directory: Cargo build output when the parent folder has Cargo.toml (also Maven when it has pom.xml)." },
      { path: "C:\\code\\hoardini\\target", gb: 52, verdict: "safe", reason: "Rust target directory: Cargo build output when the parent folder has Cargo.toml (also Maven when it has pom.xml)." },
      { path: "C:\\code\\llama.cpp\\build", gb: 6.2, verdict: "review", reason: "CMake build folder: Output of a CMake build; check nothing else lives in it." },
      { path: "C:\\code\\based\\.venv", gb: 4.1, verdict: "redownload", reason: "Project virtualenv: Per-project Python environment." },
      { path: "C:\\code\\covenant-agent\\node_modules", gb: 3.6, verdict: "redownload", reason: "node_modules: Installed npm packages; `npm install` restores them.", memory_note: "Safe to delete when the project has not been touched in 60 days; npm install brings it back." },
    ],
  },
  {
    card: "disposable",
    title: "Folders you said can go",
    kind: "delete",
    ticked: true,
    summary: "You answered Disposable for these folders.",
    items: [{ path: "C:\\code\\based\\runs", gb: 9.6, verdict: "safe", reason: "You said this folder can go." }],
  },
  {
    card: "redownload",
    title: "Re-downloadable, verified",
    kind: "delete",
    ticked: true,
    summary: "Files with an identical copy online or in your archive, checked byte for byte; they can be fetched again.",
    items: [
      {
        path: "~\\.lmstudio\\models\\llama-3.3-70b-Q4_K_M.gguf",
        gb: 42.5,
        verdict: "redownload",
        kind: "file",
        reason: "LM Studio models: identical to lmstudio-community/Llama-3.3-70B-Instruct-GGUF on Hugging Face.",
        copy_note: "identical to lmstudio-community/Llama-3.3-70B-Instruct-GGUF on Hugging Face",
      },
      { path: "~\\Downloads\\balenaEtcher-2.1.7.Setup.exe", gb: 0.16, verdict: "redownload", kind: "file", reason: "In winget as Balena.Etcher 2.1.7.", copy_note: "in winget as Balena.Etcher 2.1.7" },
    ],
  },
  {
    card: "online_only",
    title: "Online-only",
    kind: "online_only",
    ticked: true,
    summary: "Synced files nobody has opened for 90 days: the copy on this PC is freed, the files stay in the cloud and download again when opened.",
    items: [
      { path: "~\\OneDrive\\Pictures\\Camera Roll 2019", gb: 4.6, verdict: "synced", reason: "Synced to OneDrive; nothing in it opened or changed for 412 days.", copy_note: "synced to OneDrive" },
      { path: "~\\OneDrive\\Videos\\drone-2018.mp4", gb: 1.9, verdict: "synced", kind: "file", reason: "Synced to OneDrive; nothing in it opened or changed for 655 days.", copy_note: "synced to OneDrive" },
    ],
  },
  {
    card: "downloads",
    title: "Installers and archives in Downloads",
    kind: "delete",
    ticked: false,
    summary: "Disk images and installers you downloaded; each can be downloaded again.",
    items: [
      { path: "~\\Downloads\\Win11_25H2_English_x64.iso", gb: 7, verdict: "redownload", reason: "Downloaded installers and ISOs: Installers and disk images in Downloads.", kind: "file" },
      { path: "~\\Downloads\\ubuntu-24.04.3-desktop-amd64.iso", gb: 5.9, verdict: "redownload", reason: "Downloaded installers and ISOs: Installers and disk images in Downloads.", kind: "file" },
      { path: "~\\Downloads\\proxmox-ve_9.0-1.iso", gb: 1.4, verdict: "redownload", reason: "Downloaded installers and ISOs: Installers and disk images in Downloads.", kind: "file" },
      { path: "~\\Downloads\\OllamaSetup.exe", gb: 1.1, verdict: "redownload", reason: "Downloaded installers and ISOs: Installers and disk images in Downloads.", kind: "file" },
    ],
  },
  {
    card: "archive",
    title: "Send elsewhere",
    kind: "archive",
    ticked: false,
    summary: "Photos nobody has opened for 180 days: copied to the destination, checked, then removed here.",
    items: [{ path: "~\\Pictures\\2019", gb: 11.2, verdict: "review", reason: "Your photos and videos: nothing in it opened or changed for 412 days." }],
  },
  {
    card: "compress",
    title: "Compress in place",
    kind: "compress",
    format: "lzx",
    ticked: false,
    summary: "Folders Windows can compress in place (LZX): the files stay where they are and open as before.",
    items: [
      { path: "~\\AppData\\Local\\Programs\\Ollama\\lib\\ollama", gb: 4.6, verdict: "review", reason: "Windows compression would save about 2.0 GB (43% of it); the files stay usable." },
      { path: "~\\AppData\\Local\\datalab\\datalab\\Cache\\models", gb: 2.9, verdict: "review", reason: "Windows compression would save about 1.2 GB (41% of it); the files stay usable." },
    ],
  },
  {
    card: "windows",
    title: "Cleanups only Windows can do",
    kind: "system_cleanup",
    ticked: true,
    summary: "Space only Windows' own tools or an administrator can free; approving asks for administrator rights once.",
    items: [],
    ops: windowsOps,
  },
];

const LEFTOVER_PARTS: [string, number][] = [
  ["C:\\Windows\\SoftwareDistribution\\Download\\4b1f9c0e7d2a6f31", 3.4],
  ["C:\\Windows\\LiveKernelReports\\WATCHDOG", 1.1],
  ["C:\\Windows\\Temp\\{8C1F2B7A-4D0E-4F2B-9C41-6A2E0D5B7F13}", 0.9],
  ["C:\\ProgramData\\Microsoft\\Windows\\WER\\ReportArchive", 0.6],
  ["C:\\Windows\\Logs\\CBS\\CbsPersist_20260912.cab", 0.4],
];
function windowsOps(): T.ProposalItem[] {
  const parts = LEFTOVER_PARTS.map(([path, gb]) => ({ path, size: Math.round(gb * GB) }));
  const left = parts.reduce((a, x) => a + x.size, 0);
  const item = (id: number, path: string, alloc: number, reason: string, op: T.SystemOpView): T.ProposalItem => ({
    id,
    path,
    kind: op.op === "hibernation" ? "file" : "dir",
    alloc,
    verdict: "safe",
    cap: null,
    reason,
    block_reason: null,
    selected: true,
    status: "pending",
    detail: null,
    small: false,
    memory_note: null,
    op,
  });
  const view = (op: T.SystemOpView["op"], label: string, explain: string, size: number, known: boolean, more: Partial<T.SystemOpView> = {}): T.SystemOpView => ({
    op,
    label,
    explain,
    elevated: true,
    irreversible: false,
    estimate_known: known,
    size,
    ...more,
  });
  return [
    item(1, parts[0]!.path, left, `${(left / GB).toFixed(1)} GB of Windows temp files, update downloads, crash dumps and logs untouched for a day, in ${parts.length} places`,
      view("leftovers", "Windows temp files, update downloads, crash dumps and logs", "Hoardini empties Windows' own temp, update download, Delivery Optimization, crash dump and servicing log folders, which only an administrator may clear. Only files untouched for a day go; files in use and the folders themselves stay. Windows downloads again anything it still needs.", left, true, { parts })),
    item(2, "C:\\Windows\\WinSxS", 0, "WinSxS holds 19.2 GB that is not hard-linked into Windows itself; Windows decides how much of it is superseded",
      view("component_store", "Windows Update leftovers (component store)", "Windows removes superseded versions of its own components from WinSxS (DISM /StartComponentCleanup, the same as Disk Cleanup's Windows Update Cleanup). Updates installed so far can no longer be uninstalled. It can take ten minutes or more.", Math.round(19.2 * GB), false)),
    item(3, "C:\\hiberfil.sys", Math.round(17.5 * GB), "hiberfil.sys is 35.1 GB; the reduced size is about 17.6 GB (a fifth of the RAM)",
      view("hibernation", "Hibernation file at the reduced size", "Windows shrinks hiberfil.sys to the reduced size (powercfg /h /type reduced). Fast Startup keeps working; the Hibernate option goes away, so a laptop on critical battery shuts down instead of hibernating. powercfg /h /type full undoes it.", Math.round(35.1 * GB), true)),
  ];
}

const ADMIN = "Needs admin.";
const MODELS = "Model files: Hoardini proposes them once it can show where else a copy exists.";
const VM = "A virtual machine's disk: deleting it deletes the machine inside.";
const RECENT = " Something in it changed in the last 30 days.";
const ALL: T.ReviewArea["actions"] = ["delete", "dedupe", "archive", "compress"];
const area = (path: string, gb: number, label: string, category: string, verdict: T.Verdict | null, why_not: string, hint: string | null = null, actions: T.ReviewArea["actions"] = []): T.ReviewArea => ({
  path,
  size: Math.round(gb * GB),
  label,
  category,
  verdict,
  why_not,
  hint,
  unknown: category === "unknown",
  actions,
  only_copy: false,
});
const areaSeeds = (): T.ReviewArea[] =>
  [
    area("C:\\Games", 300, "Steam library", "game-library", "redownload", "Installed games: remove them through their launcher.", null, ALL),
    area("C:\\Program Files", 170, "installed programs", "system", "keep", "Installed programs: Hoardini never deletes anything here; uninstall an app to free its space."),
    area("C:\\AI", 119, "model weights", "model-weights", "review", MODELS, null, ALL),
    {
      ...area("C:\\AI\\.flm-scratch\\Qwen3.6-35B-A3B-NPU2-requant\\model.q4nx", 21, "FastFlowLM NPU model weights", "model-weights", "review", "", null, ALL),
      why_not: "Only copy we know of: no other file of this size on the indexed drives; not found on Hugging Face.",
      only_copy: true,
    },
    area("~\\AppData\\Local\\Packages\\CanonicalGroupLimited.Ubuntu24.04LTS_79rhkp1fndgsc\\LocalState\\ext4.vhdx", 121, "WSL Ubuntu disk", "vm-disk", "review", VM, null, ALL),
    area("~\\.flm\\models", 103.6, "FastFlowLM models", "model-store", "redownload", MODELS, null, ALL),
    area("C:\\code\\covenant-agent", 95, "unknown", "unknown", null, `Not identified: no catalog entry matches, and no kind of file makes up 80% of it.${RECENT}`),
    area("~\\.lmstudio\\models", 52.9, "LM Studio models", "model-store", "redownload", MODELS, null, ALL),
    area("~\\AppData\\Local\\Docker\\wsl\\disk\\docker_data.vhdx", 78.2, "Docker Desktop's WSL disk", "vm-disk", "review", VM, null, ALL),
    area("C:\\ProgramData", 43.4, "app data for every user", "app-data", "keep", "App data for every user: programs keep their shared settings and data here.", ADMIN),
    area("C:\\Windows", 25.8, "the Windows folder", "system", "keep", "The Windows folder: Hoardini never deletes anything here."),
    area("~\\.cache\\huggingface", 31, "Hugging Face cache", "model-store", "redownload", MODELS, null, ALL),
    area("C:\\hiberfil.sys", 35.1, "Hibernation file", "system", "keep", "Hibernation file: Hibernate and Fast Startup image, typically 40-75% of RAM.", "Windows recreates it at boot; turning hibernation off removes it."),
    area("C:\\code\\llama.cpp", 24.8, "unknown", "unknown", null, "Not identified: no catalog entry matches, and no kind of file makes up 80% of it."),
    area("~\\Documents", 22, "your documents", "documents", "keep", "Your documents: 88% of its bytes are documents, photos or video.", null, ["archive"]),
    area("~\\.hipfire", 21, "unknown", "unknown", null, `Not identified: no catalog entry matches, and no kind of file makes up 80% of it.${RECENT}`),
    area("C:\\pagefile.sys", 16, "Page file", "system", "keep", "Page file: Virtual-memory backing files, in use by the OS.", "Windows manages its size."),
    area("C:\\Windows\\WinSxS", 19.2, "WinSxS component store", "system", "keep", "WinSxS component store: Component store; mostly hardlinks, so its apparent size overstates real use.", "Windows can trim this itself."),
    area("C:\\$WINDOWS.~BT", 9.2, "Windows upgrade leftovers", "windows-leftover", "safe", "Windows upgrade leftovers: Files a feature update left behind.", ADMIN),
    area("~\\Pictures", 6.8, "your photos and videos", "photos", "keep", "Your photos and videos: 94% of its bytes are images or video.", null, ["archive"]),
  ].sort((a, b) => b.size - a.size);

const apps: T.RarelyUsedApp[] = [
  { id: "steam-1091500", name: "Cyberpunk 2077", publisher: "CD PROJEKT RED", source: "steam", path: "C:\\Games\\steamapps\\common\\Cyberpunk 2077", size: Math.round(71.3 * GB), last_run_ms: now() - 133 * DAY, last_run_from: "Steam LastPlayed" },
  { id: "reg-6f1c0a9be2d47c31", name: "Visual Studio Community 2022", publisher: "Microsoft Corporation", source: "registry", path: "C:\\Program Files\\Microsoft Visual Studio\\2022\\Community", size: Math.round(38.4 * GB), last_run_ms: null, last_run_from: null },
  { id: "reg-a03d5e7714c2b980", name: "HIP SDK Core", publisher: "Advanced Micro Devices, Inc.", source: "registry", path: "C:\\Program Files\\AMD\\ROCm\\6.4", size: Math.round(21.7 * GB), last_run_ms: null, last_run_from: null },
  { id: "reg-2b8e44f0c19a6d57", name: "Visual Studio Build Tools 2022", publisher: "Microsoft Corporation", source: "registry", path: "C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools", size: Math.round(9.6 * GB), last_run_ms: now() - 214 * DAY, last_run_from: "last run of c:\\program files (x86)\\microsoft visual studio\\2022\\buildtools\\common7\\tools\\vsdevcmd.bat" },
  { id: "reg-91d7c3e5a0f26b48", name: "LLVM", publisher: "LLVM", source: "registry", path: "C:\\Program Files\\LLVM", size: Math.round(4.2 * GB), last_run_ms: now() - 160 * DAY, last_run_from: "last run of c:\\program files\\llvm\\bin\\clang.exe" },
];
const uninstalled: string[] = [];

type MockReview = { id: string; created_ms: number; volume: string; scan_ms: number | null; cards: T.ReviewCardRef[]; areas: T.ReviewArea[] };
const reviews: MockReview[] = [];
const under = (root: string, p: string) => p.toLowerCase() === root.toLowerCase() || p.toLowerCase().startsWith(`${root.toLowerCase().replace(/\\$/, "")}\\`);

function buildReview(): MockReview {
  const vol = selectedDrive().root;
  const prior = reviews.filter((r) => r.volume === vol);
  const replacing = new Set(prior.flatMap((r) => [...proposals.values()].filter((p) => p.review_id === r.id && p.status === "pending" && !p.touched).map((p) => p.id)));
  const taken = new Set([...proposals.values()].filter((p) => p.status === "pending" && !replacing.has(p.id)).flatMap((p) => p.items.map((i) => i.path.toLowerCase())));
  const r: MockReview = { id: uuid(), created_ms: now(), volume: vol, scan_ms: status.index.last_refresh_ms, cards: [], areas: areaSeeds() };
  for (const seed of cardSeeds()) {
    if (seed.card === "archive" && !destinations.length) continue;
    if (seed.ops) {
      // Ops count by name: one pending in an earlier card that stays is left out (REVIEW-BUILD).
      const busy = new Set([...proposals.values()].filter((p) => p.kind === "system_cleanup" && p.review_id && p.status === "pending" && !replacing.has(p.id)).flatMap((p) => p.items.map((i) => i.op?.op)));
      const ops = seed.ops().filter((i) => !busy.has(i.op!.op));
      if (!ops.length) continue;
      ops.forEach((i, n) => (i.id = n + 1));
      const sys: T.Proposal = { id: uuid(), kind: "system_cleanup", status: "pending", total_alloc: ops.reduce((a, i) => a + i.alloc, 0), created_ms: now(), job_id: null, destination: null, format: null, summary: seed.summary, items: ops, parent_id: null, follow_up_id: null, review_id: r.id, card: seed.card, touched: false };
      proposals.set(sys.id, sys);
      r.cards.push({ card: seed.card, title: seed.title, proposal_id: sys.id });
      continue;
    }
    const kept = seed.items.filter((i) => !taken.has(i.path.toLowerCase()));
    const bytes = kept.reduce((a, i) => a + i.gb * GB, 0);
    if (!kept.length || bytes < 500 * MB) continue;
    const total = kept.reduce((a, i) => a + Math.round(i.gb * GB), 0);
    const items: T.ProposalItem[] = kept.map((i, n) => {
      const alloc = Math.round(i.gb * GB);
      const small = alloc < 50 * MB && alloc * 50 < total;
      const ticks = seed.ticked && (i.verdict === "safe" || i.verdict === "redownload" || i.verdict === "synced") && !small;
      return { id: n + 1, path: i.path, kind: i.kind ?? "dir", alloc, verdict: i.verdict, cap: null, reason: i.reason, block_reason: null, selected: ticks, status: "pending", detail: null, small, memory_note: i.memory_note ?? null, copy_note: i.copy_note ?? null };
    });
    const p: T.Proposal = {
      id: uuid(),
      kind: seed.kind,
      status: "pending",
      total_alloc: total,
      created_ms: now(),
      job_id: null,
      destination: seed.kind === "archive" ? { id: destinations[0]!.id, name: destinations[0]!.name } : null,
      format: seed.format ?? null,
      summary: seed.summary,
      items,
      parent_id: null,
      follow_up_id: null,
      review_id: r.id,
      card: seed.card,
      touched: false,
    };
    proposals.set(p.id, p);
    r.cards.push({ card: seed.card, title: seed.title, proposal_id: p.id });
  }
  r.cards.sort((a, b) => proposals.get(b.proposal_id)!.total_alloc - proposals.get(a.proposal_id)!.total_alloc);
  for (const id of replacing) {
    const p = proposals.get(id)!;
    if (p.status === "pending" && !p.touched) p.status = "superseded";
  }
  reviews.push(r);
  status.review = { id: r.id, created_ms: r.created_ms };
  dirty = true;
  return r;
}

const latestVolumeReview = () => [...reviews].reverse().find((r) => r.volume === selectedDrive().root);

let buildTimer: ReturnType<typeof setTimeout> | null = null;
const BUILD_MS = Number(process.env.MOCK_BUILD_MS ?? 2500);
function buildLater(ms = BUILD_MS): void {
  if (buildTimer) return;
  status.review_building = true;
  dirty = true;
  buildTimer = setTimeout(() => {
    buildTimer = null;
    buildReview();
    status.review_building = false;
    dirty = true;
  }, ms);
}

const LIVE: T.Proposal["status"][] = ["pending", "approved", "running"];

function carriedInto(r: MockReview): T.ReviewCardRef[] {
  const out: T.ReviewCardRef[] = [];
  for (const old of reviews.slice(0, reviews.indexOf(r))) {
    if (old.volume !== r.volume) continue;
    for (const c of old.cards) {
      let p = proposals.get(c.proposal_id);
      const seen = new Set<string>();
      while (p && !(p.touched && LIVE.includes(p.status)) && p.follow_up_id && !seen.has(p.id)) {
        seen.add(p.id);
        p = proposals.get(p.follow_up_id);
      }
      if (!p || !p.touched || !LIVE.includes(p.status)) continue;
      if (r.cards.some((x) => x.proposal_id === p!.id) || out.some((x) => x.proposal_id === p!.id)) continue;
      out.push({ card: c.card, title: c.title, proposal_id: p.id, carried: true });
    }
  }
  return out;
}

function rebuildIfUntouched(): void {
  const last = latestVolumeReview();
  if (!last || last.scan_ms === status.index.last_refresh_ms) return;
  if ([...proposals.values()].some((p) => p.review_id === last.id && (p.touched || p.status !== "pending"))) return;
  buildReview();
}

const extras = (root: string) => ({
  building: status.review_building ?? false,
  apps: apps.filter((a) => under(root, a.path)),
});

function reviewView(r: MockReview): T.ReviewView {
  const cards = [...r.cards, ...carriedInto(r)].sort((a, b) => proposals.get(b.proposal_id)!.total_alloc - proposals.get(a.proposal_id)!.total_alloc);
  const props: T.Proposal[] = [];
  for (const c of cards)
    for (let p = proposals.get(c.proposal_id); p && !props.includes(p); p = p.follow_up_id ? proposals.get(p.follow_up_id) : undefined) props.push(p);
  const ready = props.filter((p) => p.status === "pending").flatMap((p) => p.items).filter((i) => i.selected && i.status === "pending" && !i.block_reason).reduce((a, i) => a + i.alloc, 0);
  const d = selectedDrive();
  return {
    review: { id: r.id, created_ms: r.created_ms, volume: r.volume },
    header: { root: r.volume, used_bytes: d.total_bytes - d.free_bytes, free_bytes: d.free_bytes, ready_bytes: ready },
    cards,
    areas: r.areas,
    proposals: Object.fromEntries(props.map((p) => [p.id, p])),
    ...extras(r.volume),
  };
}

function firstView(): T.ReviewView {
  const d = selectedDrive();
  return { review: null, header: { root: d.root, used_bytes: d.total_bytes - d.free_bytes, free_bytes: d.free_bytes, ready_bytes: 0 }, cards: [], areas: [], proposals: {}, ...extras(d.root) };
}

const CONTAINERS = new Set(["c:\\users", "~", "~\\appdata", "~\\appdata\\local", "~\\appdata\\roaming", "~\\.lmstudio", "c:\\programdata", "c:\\program files", "c:\\windows"]);

function answerLocation(path: string, answer: T.LocationAnswer): T.Memory {
  const table: Record<T.LocationAnswer, { label: string; verdict: T.Verdict; actions: T.ReviewArea["actions"]; text: string }> = {
    keep: { label: "yours", verdict: "keep", actions: ["archive", "compress"], text: "You keep this folder; it may still be moved or compressed." },
    disposable: { label: "disposable", verdict: "safe", actions: ["delete", "dedupe", "archive", "compress"], text: "You said this folder can go." },
    "move-elsewhere": { label: "move elsewhere", verdict: "keep", actions: ["archive"], text: "You want this folder moved to another drive, not deleted." },
    "never-touch": { label: "never touch", verdict: "keep", actions: [], text: "Never touch this folder." },
  };
  const a = table[answer];
  for (const r of reviews)
    for (const area of r.areas.filter((x) => x.path.toLowerCase() === path.toLowerCase()))
      Object.assign(area, { label: a.label, category: area.category === "unknown" ? "user" : area.category, verdict: a.verdict, actions: a.actions, why_not: a.text, unknown: false, user: true });
  const m: T.Memory = { id: ++memSeq, scope: "path", key: path, kind: "verdict", text: a.text, source: "user", confidence: null, created_ms: now(), updated_ms: now(), review_after_ms: null, pinned: true, last_used_ms: now(), forgotten_ms: null, detail: { category: "user", label: a.label, verdict: a.verdict, actions: a.actions, by: "user", assessed_ms: now() } };
  memories.push(m);
  if (answer === "never-touch") memories.push({ ...m, id: ++memSeq, kind: "protect", text: "Never touch: your answer to What is this?", detail: null });
  return m;
}

const extOf = (name: string) => (name.lastIndexOf(".") > 0 ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "");
function typesOf(path: string): T.TypeRow[] | null {
  const root = findNode(path);
  if (!root) return null;
  const by = new Map<string, { bytes: number; files: number }>();
  const small = { bytes: 0, files: 0 };
  const add = (ext: string, bytes: number, files: number) => {
    const e = by.get(ext) ?? { bytes: 0, files: 0 };
    e.bytes += bytes;
    e.files += files;
    by.set(ext, e);
  };
  const walk = (n: TreeNode) => {
    if (n.kind === "file") return add(extOf(n.name), n.size, 1);
    // Made-up contents depend on what was opened before, so types use the folder's mix and stay the same.
    if (n.grown) return guess(n);
    if (n.small) {
      small.bytes += n.small.bytes;
      small.files += n.small.count;
    }
    if (n.kids) return n.kids.forEach(walk);
    guess(n);
  };
  const guess = (n: TreeNode) => {
    const exts = hintFor(n.path);
    const shares = exts.length === 1 ? [0.985] : exts.length === 2 ? [0.66, 0.325] : [0.6, 0.265, 0.12];
    exts.forEach((e, i) => add(e, Math.round(n.size * shares[i]!), Math.max(1, Math.round((n.files * shares[i]!) / 4))));
    small.bytes += Math.round(n.size * 0.015);
    small.files += Math.round(n.files * 0.7);
  };
  walk(root);
  const fixed: Record<string, string> = { gguf: "model-weights", safetensors: "model-weights", pt: "model-weights", jpg: "photos", heic: "photos", mp4: "photos", pdf: "documents", docx: "documents" };
  const rows: T.TypeRow[] = [...by.entries()].map(([ext, v]) => {
    const m = memories.find((x) => x.scope === "glob" && x.key.toLowerCase() === `**\\*.${ext}` && x.detail);
    const category = m?.detail?.category ?? fixed[ext];
    return { ext, bytes: v.bytes, files: v.files, ...(m ? { label: m.text, app: m.detail?.app, use: m.detail?.use } : {}), ...(category ? { category } : {}) };
  });
  if (small.bytes > 0) rows.push({ ext: null, small: true, bytes: small.bytes, files: small.files });
  return rows.sort((a, b) => b.bytes - a.bytes);
}

// ---- AG-UI run ----
const enc = new TextEncoder();
type Sse = { send: (e: Record<string, unknown>) => void; closed: () => boolean };

async function say(s: Sse, text: string) {
  const messageId = uuid();
  s.send({ type: "TEXT_MESSAGE_START", messageId, role: "assistant" });
  for (const chunk of text.match(/[\s\S]{1,18}(\s|$)/g) ?? [text]) {
    if (s.closed()) return messageId;
    s.send({ type: "TEXT_MESSAGE_CONTENT", messageId, delta: chunk });
    await sleep(35);
  }
  s.send({ type: "TEXT_MESSAGE_END", messageId });
  return messageId;
}

async function readTool(s: Sse, name: string, args: unknown, result: string, ms = 600) {
  const toolCallId = uuid();
  s.send({ type: "TOOL_CALL_START", toolCallId, toolCallName: name, parentMessageId: uuid() });
  s.send({ type: "TOOL_CALL_ARGS", toolCallId, delta: JSON.stringify(args) });
  s.send({ type: "TOOL_CALL_END", toolCallId });
  await sleep(ms);
  s.send({ type: "TOOL_CALL_RESULT", messageId: uuid(), toolCallId, content: result, role: "tool" });
}

// Item questions on Home: one short thread per item (AGENT-ASK).
const itemThreads = new Map<string, { id: string; role: "user" | "assistant"; content: string }[]>();

async function itemRun(threadId: string, runId: string, text: string, s: Sse) {
  const msgs = itemThreads.get(threadId) ?? [];
  itemThreads.set(threadId, msgs);
  msgs.push({ id: uuid(), role: "user", content: text });
  status.running_runs += 1;
  dirty = true;
  s.send({ type: "RUN_STARTED", threadId, runId });
  try {
    const about = /^About (.*?) \(/.exec(text)?.[1];
    await readTool(s, "inspect", { path: about ?? "" }, "folder; 2 big subfolders", 600);
    // The "find out" answer quotes long paths, which must wrap inside the sheet.
    const answer = /saved/i.test(text)
      ? "They're the clips you saved on purpose. **Keep them** if they matter to you; the rest the car overwrites by itself."
      : /what is this/i.test(text)
        ? `**What it is**\n\n\`${about ?? ""}\` holds big files. Windows keeps per-user scratch space under \`C:\\Users\\<username>\\AppData\\Local\\Temp\\claude\\c--code-openflowlm-next\\bf29ccdf-72d3-40c3-b182-8323b90119e3\`.\n\n\`\`\`\nC:\\Users\\tester\\AppData\\Local\\Temp\\claude\\c--code-openflowlm-next\\bf29ccdf-72d3-40c3-b182-8323b90119e3\\scratchpad\\model.gguf\n\`\`\`\n\n**Recommendation: delete it** if nothing is running from it.`
        : "It's a folder of big files you made yourself. Nothing rebuilds it, so delete it only if you're sure you won't need it again.";
    const id = await say(s, answer);
    msgs.push({ id, role: "assistant", content: answer });
    s.send({ type: "RUN_FINISHED", threadId, runId });
  } finally {
    status.running_runs -= 1;
    dirty = true;
  }
}

// ---- HTTP ----
const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status, headers: { "content-type": "application/json" } });
const err = (message: string, code = 400) => json({ error: message }, code);

function sseResponse(start: (s: Sse) => Promise<void> | void) {
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
      if (seg[1] === "uninstalled") return json(uninstalled);
      if (seg[1]) break;
      const b = await body<{ fill?: number; running_runs?: number; idle?: boolean; next_job?: NextJob; rebuild?: boolean | { ms: number } }>();
      if (b.rebuild === true) buildReview();
      else if (b.rebuild) buildLater(b.rebuild.ms);
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
      if (seg[1] === "children") {
        const p = url.searchParams.get("path");
        if (!p?.trim()) return err("path is required");
        const page = childrenOf(p, Math.min(1000, Math.max(1, Number(url.searchParams.get("limit") ?? 50))));
        if (!page) return err(`${p} in the index not found`, 404);
        if (findNode(p)!.kind === "file") return err(`${p} is not a folder`);
        return json(page);
      }
      if (seg[1] === "types") {
        const p = url.searchParams.get("path");
        if (!p?.trim()) return err("path is required");
        const n = findNode(p);
        if (!n) return err(`${p} in the index not found`, 404);
        if (n.kind === "file") return err(`${p} is not a folder`);
        return json(typesOf(p));
      }
      if (seg[1] === "refresh") {
        if (status.index.state !== "idle") return json({ started: false });
        startRefresh("fast");
        return json({ started: true });
      }
      break;
    case "agent": {
      if (seg[1] !== "run" || method !== "POST") break;
      const input = await body<{ runId: string; threadId?: string; messages: { id?: string; role: string; content?: unknown }[] }>();
      const threadId = input.threadId;
      if (!threadId?.startsWith("item:")) return err("threadId must be an item thread");
      const text = String(input.messages[input.messages.length - 1]?.content ?? "");
      return sseResponse((s) => itemRun(threadId, input.runId, text, s));
    }
    case "thread": {
      if (seg[1] || method !== "GET") break;
      const id = url.searchParams.get("id");
      if (!id?.startsWith("item:")) return err("unknown thread");
      return json({ id, messages: itemThreads.get(id) ?? [] });
    }
    case "proposals": {
      const p = proposals.get(seg[1]!);
      if (!p) return err("proposal not found", 404);
      if (!seg[2]) return json(p);
      if (seg[2] === "selection" && method === "PUT") {
        if (p.status !== "pending") return err(`proposal ${p.id} is ${p.status}, not pending`, 409);
        const { item_ids } = await body<{ item_ids: number[] }>();
        const bad = (item_ids ?? []).find((i) => !p.items.some((x) => x.id === i && x.status === "pending"));
        if (bad != null) return err(`item ${bad} is not part of proposal ${p.id}`, 409);
        for (const it of p.items) if (it.status === "pending") it.selected = item_ids.includes(it.id);
        p.touched = true;
        dirty = true;
        return json(p);
      }
      if (seg[2] === "approve") {
        if (p.status !== "pending") return err(`proposal is ${p.status}`, 409);
        const { item_ids } = await body<{ item_ids: number[] }>();
        if (!item_ids?.length) return err("nothing selected");
        if (nextJob?.startDelayMs) await sleep(nextJob.startDelayMs);
        if (nextJob?.rejectApprove) {
          nextJob = null;
          return err("could not start the job", 409);
        }
        p.status = "approved";
        p.touched = true;
        const job = runJob(p, item_ids);
        return json({ approved: true, items: item_ids, job_id: job.id });
      }
      break;
    }
    case "review": {
      if (status.index.last_refresh_ms == null) return err("index not found", 404);
      if (seg[1] === "open" && method === "POST") {
        const last = latestVolumeReview();
        const stale = !last || now() - last.created_ms > SESSION_GAP_MS || (status.index.last_refresh_ms ?? 0) !== (last.scan_ms ?? 0);
        if (stale) buildLater();
        return json(last ? reviewView(last) : firstView());
      }
      if (seg[1] === "run" && method === "POST") {
        await sleep(Number(process.env.MOCK_RUN_MS ?? 0));
        return json(reviewView(buildReview()));
      }
      if (seg[1] || method !== "GET") break;
      const id = url.searchParams.get("id");
      const r = id ? reviews.find((x) => x.id === id) : latestVolumeReview();
      return r ? json(reviewView(r)) : err("review not found", 404);
    }
    case "apps": {
      const app = apps.find((a) => a.id === seg[1]);
      if (seg[2] !== "uninstall" || method !== "POST") break;
      if (!app) return err("app not found", 404);
      uninstalled.push(app.id);
      console.log(`uninstall ${app.name}`);
      return json({ started: true, id: app.id, name: app.name });
    }
    case "locations": {
      if (seg[1] === "forget" && method === "POST") {
        const b = await body<{ path?: string }>();
        if (!b.path?.trim()) return err("path is required");
        const before = memories.length;
        for (let i = memories.length - 1; i >= 0; i--) if (memories[i]!.source === "user" && memories[i]!.scope === "path" && norm(memories[i]!.key) === norm(b.path)) memories.splice(i, 1);
        for (const r of reviews) for (const a of r.areas) if (norm(a.path) === norm(b.path)) Object.assign(a, { user: false, label: "unknown", category: "unknown", verdict: null, actions: [], why_not: "Not identified: no catalog entry matches, and no kind of file makes up 80% of it.", unknown: true });
        dirty = true;
        return json({ forgot: before - memories.length });
      }
      if (seg[1] !== "answer" || method !== "POST") break;
      const b = await body<{ path?: string; answer?: T.LocationAnswer }>();
      if (!b.path) return err("path is required");
      if (!b.answer || !["keep", "disposable", "move-elsewhere", "never-touch"].includes(b.answer)) return err("invalid request body: unknown answer");
      if (CONTAINERS.has(norm(b.path)) || /^[a-z]:$/.test(norm(b.path))) return err(`${b.path} holds other folders; answer for a folder inside it`);
      dirty = true;
      return json(answerLocation(b.path, b.answer));
    }
    case "act": {
      if (method !== "POST") break;
      const b = await body<{ kind?: string; paths?: string[]; destination_id?: string }>();
      const kind = b.kind as T.ProposalKind | undefined;
      if (!kind || !["delete", "compress", "online_only", "archive"].includes(kind)) return err("kind must be delete, compress, online_only or archive");
      if (kind === "archive" && !b.destination_id) return err("moving needs a destination_id");
      const paths = (b.paths ?? []).map((x) => x.trim()).filter(Boolean);
      if (!paths.length) return err("paths is required");
      // Like the gate: a protect memory (a Never touch answer) holds a path back.
      const held = (x: string) => memories.some((m) => m.kind === "protect" && !m.forgotten_ms && (norm(x) === norm(m.key) || norm(x).startsWith(`${norm(m.key)}\\`)));
      const blocked = paths.filter(held).map((x) => ({ path: x, reason: `${x} is protected by your note` }));
      const go = paths.filter((x) => !held(x));
      if (!go.length) return json({ proposal_id: null, job_id: null, blocked });
      const sizeOf = (x: string) => reviews.flatMap((r) => r.areas).find((a) => norm(a.path) === norm(x))?.size ?? findNode(x)?.size ?? GB;
      const items: T.ProposalItem[] = go.map((x, i) => ({ id: i + 1, path: x, kind: "dir", alloc: Math.round(sizeOf(x)), verdict: null, cap: null, reason: null, block_reason: null, selected: true, status: "pending", detail: null, small: false, memory_note: null }));
      const p: T.Proposal = { id: uuid(), kind, status: "approved", total_alloc: items.reduce((a, i) => a + i.alloc, 0), created_ms: now(), job_id: null, destination: null, format: kind === "compress" ? "lzx" : null, summary: "Chosen on Home", items, parent_id: null, follow_up_id: null };
      proposals.set(p.id, p);
      const job = runJob(p, items.map((i) => i.id));
      // The real server's next review no longer lists what went; drop it at once here.
      for (const r of reviews) r.areas = r.areas.filter((a) => !go.some((x) => norm(x) === norm(a.path)));
      dirty = true;
      return json({ proposal_id: p.id, job_id: job.id, blocked });
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
      if (seg[2] || method !== "PUT") break;
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
      return json({ path: seg[1] === "pick-model" ? "D:\\models\\my-model.gguf" : "D:\\Archive" });
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
    case "settings":
      if (method === "PUT") {
        const patch = await body<T.SettingsPatch>();
        const oc = patch.online_checks;
        delete patch.online_checks;
        Object.assign(settings, patch, { index_size_bytes: settings.index_size_bytes });
        if (oc) settings.online_checks = { asked: oc.asked ?? settings.online_checks.asked, services: { ...settings.online_checks.services, ...oc.services } };
      }
      return json(settings);
    case "presence":
      return json({ ok: true });
    case "archive":
      return json([]);
    case "builtin":
      if (seg[1] === "install") return json(installBuiltin((await body<{ model_id: string }>()).model_id));
      if (method === "DELETE") {
        const freed = builtin.runtime.size + (builtin.model?.custom ? 0 : (builtin.model?.size ?? 0));
        Object.assign(builtin, { installed: false, running: false, model: null, provider_id: null, runtime: cpuRuntime });
        cpuRuntime.installed = false;
        gpuRuntime.installed = false;
        for (const c of builtin.candidates) c.installed = false;
        return json({ ok: true, freed_bytes: freed });
      }
      return json(builtin);
  }
  return err(`no mock for ${method} ${path}`, 404);
}

const cpuRuntime: T.BuiltinRuntime = { id: "llama.cpp", version: "b11503", size: 19_483_533, installed: false, license: "MIT", license_url: "https://github.com/ggml-org/llama.cpp/blob/b11503/LICENSE" };
const gpuRuntime: T.BuiltinRuntime = { id: "llama.cpp-vulkan", version: "b11503", size: 33_425_617, installed: false, license: "MIT", license_url: "https://github.com/ggml-org/llama.cpp/blob/b11503/LICENSE" };
const builtin: T.BuiltinView = {
  installed: false,
  running: false,
  runtime: cpuRuntime,
  custom_runtime: gpuRuntime,
  model: null,
  candidates: [
    { id: "qwen3.5-9b", name: "Qwen3.5 9B", size: 5_577_290_080, license: "Apache-2.0", license_url: "https://www.apache.org/licenses/LICENSE-2.0", installed: false, recommended: false, needs_gpu: true, runtime: gpuRuntime },
    { id: "granite-4.1-3b", name: "Granite 4.1 3B", size: 1_998_372_160, license: "Apache-2.0", license_url: "https://www.apache.org/licenses/LICENSE-2.0", installed: false, recommended: true, needs_gpu: false, runtime: cpuRuntime },
  ],
  free_bytes: 62 * GB,
  recommended: "granite-4.1-3b",
  install_job: null,
  provider_id: null,
  threads: 10,
  context: 8192,
};

function installBuiltin(modelId: string): T.Job {
  const file = modelId.startsWith("file:") ? modelId.slice(5) : null;
  const pick: T.BuiltinModel = file
    ? { id: modelId, name: file.split(/[\/]/).pop()!, size: 4 * GB, license: "your file", license_url: null, installed: true, recommended: false, needs_gpu: false, runtime: gpuRuntime, custom: true, path: file }
    : builtin.candidates.find((c) => c.id === modelId)!;
  const rt = pick.runtime;
  const total = (rt.installed ? 0 : rt.size) + (file || pick.installed ? 0 : pick.size);
  const job: T.Job = { id: uuid(), kind: "builtin_install", proposal_id: null, status: "running", progress: { done_bytes: 0, total_bytes: total, done_items: 0, total_items: 2, current: `llama.cpp ${rt.version}` }, freed_bytes: 0, error: null, created_ms: now(), finished_ms: null };
  builtin.install_job = job;
  emitJob(job);
  void (async () => {
    for (let s = 1; s <= 20; s++) {
      await sleep(300);
      job.progress.done_bytes = (total * s) / 20;
      job.progress.current = s < 2 ? `llama.cpp ${rt.version}` : pick.name;
      emitJob(job);
    }
    Object.assign(job, { status: "done", finished_ms: now(), progress: { ...job.progress, done_items: 2, current: null } });
    rt.installed = true;
    pick.installed = true;
    Object.assign(builtin, { installed: true, model: pick, runtime: rt, provider_id: "builtin" });
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
