export type Job = {
  id: string;
  kind: string;
  proposal_id: string | null;
  status: "queued" | "running" | "done" | "failed" | "cancelled";
  progress: { done_bytes: number; total_bytes: number; done_items: number; total_items: number; current: string | null };
  freed_bytes: number;
  error: string | null;
  created_ms: number;
  finished_ms: number | null;
};

export type Status = {
  volumes: { root: string; total_bytes: number; free_bytes: number }[];
  index: {
    state: "idle" | "refreshing";
    mode: "background" | "fast" | null;
    progress: { phase: string; dirs: number; files: number; bytes: number; fraction: number | null } | null;
    last_refresh_ms: number | null;
    generation: number;
  };
  jobs: Job[];
  vanished_total_bytes: number;
  active_provider: { id: string; name: string; model: string | null } | null;
  running_runs: number;
};

export type StatusEvent = ({ type: "status" } & Status) | ({ type: "job" } & Job);

export type IndexSummary = {
  age_ms: number;
  free_delta_bytes: number;
  movers: { path: string; delta_bytes: number }[];
  duplicate_hint: { bytes: number; count: number } | null;
};

export type Verdict = "safe" | "redownload" | "review" | "keep";
export type ProposalKind = "delete" | "archive" | "compress" | "dedupe";
export type ProposalItem = {
  id: number;
  path: string;
  kind: "dir" | "file" | "link";
  alloc: number;
  verdict: Verdict | null;
  cap: Verdict | null;
  reason: string | null;
  block_reason: string | null;
  selected: boolean;
  status: "pending" | "blocked" | "skipped" | "done" | "failed";
  detail: string | null;
};
export type Proposal = {
  id: string;
  kind: ProposalKind;
  status: "pending" | "approved" | "declined" | "running" | "done" | "partial" | "failed";
  total_alloc: number;
  created_ms: number;
  job_id: string | null;
  destination: { id: string; name: string } | null;
  format: string | null;
  dedupe?: { group_id: number; keep: string; mode: "hardlink" | "delete" } | null;
  summary: string;
  items: ProposalItem[];
};

export type Memory = {
  id: number;
  scope: "path" | "glob" | "global";
  key: string;
  kind: "fact" | "preference" | "protect" | "verdict";
  text: string;
  source: string;
  confidence: number | null;
  created_ms: number;
  updated_ms: number;
  review_after_ms: number | null;
  pinned: boolean;
  last_used_ms: number | null;
  forgotten_ms: number | null;
};
export type MemoryInput = Partial<Pick<Memory, "scope" | "key" | "kind" | "text" | "pinned">>;

export type Caps = { tools: boolean; streaming: boolean; streamed_tool_calls: boolean; tool_choice: boolean };
export type ProviderConfig = {
  id?: string;
  name: string;
  base_url: string;
  model: string | null;
  api_key: string | null;
  context_window: number;
  timeout_s: number;
  max_tool_steps: number;
  small: boolean;
  redact_paths: boolean;
  params: { temperature?: number; max_tokens?: number; top_p?: number };
  headers: Record<string, string>;
  extra_body: Record<string, unknown>;
};
export type ProviderView = ProviderConfig & { id: string; is_active: boolean; has_key: boolean; caps: Caps | null };
export type TestReport = {
  ok: boolean;
  error: string | null;
  models: string[];
  model_found: boolean | null;
  suggestions: string[];
  caps: Caps | null;
  latency_ms: number | null;
};
export type Preset = { id: string; name: string; base_url: string; needs_key: boolean; default_model: string | null; key_url: string | null };
export type LocalServer = { id: string; name: string; base_url: string; models: string[] };

export type Destination = {
  id: string;
  name: string;
  kind: "local" | "smb" | "rclone";
  config: Record<string, unknown>;
  created_ms: number;
};
export type DestinationInput = { name: string; kind: Destination["kind"]; config: Record<string, unknown> };
export type DestinationTest = { ok: boolean; free_bytes?: number; error?: string };
export type RcloneRuntime = { installed: boolean; version: string; size: number; license_url: string };
export type RcloneAuthorizeInput = { type: string; client_id_ref?: string; client_secret_ref?: string };
export type ArchiveEntry = {
  id: number;
  original_path: string;
  destination_id: string;
  dest_uri: string;
  size: number;
  archived_ms: number;
  restored_ms: number | null;
};

export type SearchKind = "tavily" | "brave" | "exa" | "serper" | "searxng";
export type SearchProviderView = {
  kind: SearchKind;
  is_active: boolean;
  has_key: boolean;
  base_url?: string | null;
  api_key?: string | null;
};
export type SearchProviderInput = { api_key?: string | null; base_url?: string | null };
export type SearchTest = { ok: boolean; error?: string | null; latency_ms?: number | null };

export type HistoryPoint = {
  finished_ms: number;
  total_bytes: number;
  free_bytes: number;
  used_bytes: number;
  indexed_bytes: number;
};

export type Settings = {
  threshold_bytes: number;
  redact_paths: boolean;
  web_search_enabled: boolean;
  indexer_workers: number;
  indexer_memory_cap_mb: number;
  builtin_idle_minutes: number;
  index_size_bytes: number;
};

export type ThreadSummary = { id: string; title: string; updated_ms: number };
export type ThreadDetail = {
  id: string;
  title: string;
  messages: unknown[];
  proposals: Record<string, Proposal>;
};

export type ApproveResponse = { approved: true; items: number[]; job_id: string };
export type DeclineResponse = { approved: false };

export type BuiltinModel = {
  id: string;
  name: string;
  size: number;
  license: string;
  license_url: string;
  installed: boolean;
  recommended: boolean;
};
export type BuiltinView = {
  installed: boolean;
  running: boolean;
  runtime: { version: string; size: number; installed: boolean; license: string; license_url: string };
  model: BuiltinModel | null;
  candidates: BuiltinModel[];
  free_bytes: number;
  recommended: string | null;
  install_job: Job | null;
  provider_id: string | null;
  threads: number;
  context: number;
};
