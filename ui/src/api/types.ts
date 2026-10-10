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

export type VolumeView = {
  root: string;
  label: string;
  fs: string;
  total_bytes: number;
  free_bytes: number;
  watched: boolean;
  selected: boolean;
  connected: boolean;
  has_journal: boolean;
};
export type VolumeChoice = { watched: string[]; selected: string };

export type Status = {
  volumes: VolumeView[];
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
  review?: { id: string; created_ms: number } | null;
  review_building?: boolean;
};

export type StatusEvent = ({ type: "status" } & Status) | ({ type: "job" } & Job);

export type IndexSummary = {
  age_ms: number;
  free_delta_bytes: number;
  movers: { path: string; delta_bytes: number }[];
  duplicate_hint: { bytes: number; count: number } | null;
};

export type Verdict = "safe" | "redownload" | "synced" | "review" | "keep";
export type ProposalKind = "delete" | "archive" | "compress" | "dedupe" | "system_cleanup" | "online_only";
export type SystemOpView = {
  op: "component_store" | "restore_points" | "hibernation" | "compact_vhdx" | "recycle_bin" | "leftovers";
  volume?: string;
  path?: string;
  label: string;
  explain: string;
  elevated: boolean;
  irreversible: boolean;
  estimate_known: boolean;
  size: number | null;
  parts?: { path: string; size: number }[];
};
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
  small: boolean;
  memory_note: string | null;
  op?: SystemOpView;
  copy_note?: string | null;
};
export type RarelyUsedApp = {
  id: string;
  name: string;
  publisher: string | null;
  source: "registry" | "steam";
  path: string;
  size: number;
  last_run_ms: number | null;
  last_run_from: string | null;
};
export type Proposal = {
  id: string;
  kind: ProposalKind;
  status: "pending" | "approved" | "declined" | "running" | "done" | "partial" | "failed" | "superseded";
  total_alloc: number;
  created_ms: number;
  job_id: string | null;
  destination: { id: string; name: string } | null;
  format: string | null;
  dedupe?: { group_id: number; keep: string; mode: "hardlink" | "delete" } | null;
  summary: string;
  items: ProposalItem[];
  parent_id: string | null;
  follow_up_id: string | null;
  learned?: Learned[];
  review_id?: string | null;
  card?: string | null;
  touched?: boolean;
};
export type Learned = { memory_id: number; text: string; url?: string; confirmed?: boolean; pinned?: boolean };

export type ReviewArea = {
  path: string;
  size: number;
  label: string;
  category: string;
  verdict: Verdict | null;
  why_not: string;
  hint: string | null;
  unknown: boolean;
  actions: ("delete" | "dedupe" | "archive" | "compress")[];
  only_copy?: boolean;
};
export type ReviewCardRef = { card: string; title: string; proposal_id: string; carried?: boolean };
export type ReviewView = {
  review: { id: string; created_ms: number; volume: string; scope_path: string | null } | null;
  header: { root: string; used_bytes: number | null; free_bytes: number | null; ready_bytes: number };
  cards: ReviewCardRef[];
  areas: ReviewArea[];
  proposals: Record<string, Proposal>;
  building?: boolean;
  apps?: RarelyUsedApp[];
  online?: OnlineState;
};
export type OnlineState = { asked: boolean; waiting: number };
export type LocationAnswer = "keep" | "disposable" | "move-elsewhere" | "never-touch";

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
  detail?: MemoryDetail | null;
};
export type MemoryDetail = {
  category?: string;
  label?: string;
  verdict?: Verdict;
  actions?: ("delete" | "dedupe" | "archive" | "compress")[];
  app?: string;
  use?: string;
  copy_hint?: string;
  size_at?: number;
  assessed_ms?: number;
  by?: string;
  cleaned?: unknown[];
  kept?: unknown[];
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

export type IndexSpeed = "lean" | "fast";
export type OnlineService = "huggingface" | "winget";
export type OnlineChecks = { asked: boolean; services: Record<OnlineService, boolean> };
export type Settings = {
  index_speed: IndexSpeed;
  builtin_idle_minutes: number;
  index_size_bytes: number;
  online_checks: OnlineChecks;
};
export type SettingsPatch = Partial<Omit<Settings, "index_size_bytes" | "online_checks">> & {
  online_checks?: { asked?: boolean; services?: Partial<Record<OnlineService, boolean>> };
};

export type CopyConfidence = "identical" | "same-name-and-size" | "claimed" | "none";
export type CopyCheckResult = {
  check: string;
  locator: string | null;
  confidence: CopyConfidence;
  checked_ms: number;
  detail: Record<string, unknown>;
  note: string;
};
export type CopyChecks = { path: string; results: CopyCheckResult[]; only_copy: boolean; note: string | null };

export type ThreadDetail = {
  id: "main";
  messages: unknown[];
  proposals: Record<string, Proposal>;
  window_start_id: string | null;
  has_hidden: boolean;
  fresh: boolean;
  earlier: number;
  showing_all: boolean;
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

export type More = { count: number; bytes: number };
export type FolderRow = { path: string; size: number; files: number; denied: boolean };
export type FileRow = { path: string; size: number; modified_ms: number | null; accessed_ms: number | null };
export type ChangeRow = { kind: "dir" | "file"; path: string; delta: number; before: number | null; after: number | null };
export type FoldersData = { v: 1; kind: "folders"; under: string | null; depth: number; view?: "list" | "map"; as_of_ms: number | null; items: FolderRow[]; more: More };
export type FilesData = {
  v: 1;
  kind: "files";
  filters: { under: string | null; min_size: number | null; ext: string | null; not_used_for_days: number | null };
  as_of_ms: number | null;
  items: FileRow[];
  more: More;
};
export type ChangesData = {
  v: 1;
  kind: "changes";
  volume: string;
  from: { id: number; ms: number };
  to: { id: number; ms: number };
  free_delta: number | null;
  indexed_delta: number;
  free_before: number | null;
  free_after: number | null;
  grew: ChangeRow[];
  new: ChangeRow[];
  shrank: ChangeRow[];
  removed: ChangeRow[];
  omitted: { grew: number; new: number; shrank: number; removed: number };
  range_ms: [number, number];
};
export type PillKind = "ask" | "clean_up" | "explore" | "what_is";
export type Pill = { label: string; kind: PillKind; path?: string };
export type PillsData = { v: 1; kind: "pills"; items: Pill[] };
export type LearnedData = { v: 1; kind: "learned"; items: Learned[] };
export type CardData = FoldersData | FilesData | ChangesData;
export type ToolData = CardData | PillsData | LearnedData;

export type ChildRow = {
  path: string;
  name: string;
  kind: "dir" | "file" | "link";
  size: number;
  files: number;
  mtime_ms: number | null;
  denied: boolean;
  has_children: boolean;
};
export type Children = { path: string; size: number; files: number; children: ChildRow[]; small: More; more: More };

export type TagState = "proposed" | "review" | "keep" | "system" | "user" | "unknown";
export type MapTag = { state: TagState; label?: string; verdict?: Verdict };
export type MapNode = {
  path: string;
  name: string;
  kind: "dir" | "file" | "link" | "small";
  size: number;
  files: number;
  ext?: string;
  children?: MapNode[];
  more?: More;
  tag: MapTag;
};
export type IndexMap = { tree: MapNode; ancestors: { path: string; name: string }[]; min_bytes: number; nodes: number; capped: boolean };
export type TypeRow = {
  ext: string | null;
  bytes: number;
  files: number;
  small?: boolean;
  other?: number;
  label?: string;
  app?: string;
  use?: string;
  category?: string;
};
