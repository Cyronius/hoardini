import type * as T from "./types";

const TOKEN_KEY = "hoardini.token";
let token: string | null = null;

function bootstrapToken(): string | null {
  const m = /[#&]token=([^&]+)/.exec(location.hash);
  if (m) {
    token = decodeURIComponent(m[1]!);
    try {
      sessionStorage.setItem(TOKEN_KEY, token);
    } catch {
      // storage disabled; memory copy still works for this page load
    }
    history.replaceState(null, "", location.pathname + location.search);
    return token;
  }
  try {
    token = sessionStorage.getItem(TOKEN_KEY);
  } catch {
    token = null;
  }
  // Only the mock/dev server accepts this; a real launch always carries a fragment token.
  if (!token && import.meta.env.DEV) token = "dev";
  return token;
}
bootstrapToken();

export const getToken = () => token;

export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
  ) {
    super(message);
  }
}

export async function api<R>(path: string, init: { method?: string; body?: unknown } = {}): Promise<R> {
  const res = await fetch(path, {
    method: init.method ?? (init.body !== undefined ? "POST" : "GET"),
    headers: {
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...(init.body !== undefined ? { "Content-Type": "application/json" } : {}),
    },
    body: init.body !== undefined ? JSON.stringify(init.body) : undefined,
  });
  const text = await res.text();
  let data: unknown = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = text;
    }
  }
  if (!res.ok) {
    const msg = (data as { error?: string } | null)?.error ?? (typeof data === "string" && data ? data : res.statusText);
    throw new ApiError(msg, res.status);
  }
  return data as R;
}

const enc = encodeURIComponent;
const put = <R>(path: string, body: unknown) => api<R>(path, { method: "PUT", body });
const del = <R = { ok: true }>(path: string) => api<R>(path, { method: "DELETE" });

export const routes = {
  status: () => api<T.Status>("/api/status"),
  refresh: (mode: "background" | "fast") => api<{ started: boolean }>("/api/index/refresh", { body: { mode } }),
  summary: () => api<T.IndexSummary>("/api/index/summary"),
  cancelRun: (runId: string) => api<{ ok: true }>(`/api/agent/runs/${enc(runId)}/cancel`, { method: "POST", body: {} }),

  thread: () => api<T.ThreadDetail>("/api/thread"),
  startFresh: () => api<{ ok: true }>("/api/thread/fresh", { body: {} }),
  clearHistory: () => del("/api/thread"),

  proposal: (id: string) => api<T.Proposal>(`/api/proposals/${enc(id)}`),
  approve: (id: string, item_ids: number[]) =>
    api<T.ApproveResponse>(`/api/proposals/${enc(id)}/approve`, { body: { item_ids } }),
  decline: (id: string) => api<T.DeclineResponse>(`/api/proposals/${enc(id)}/decline`, { body: {} }),
  jobs: () => api<T.Job[]>("/api/jobs"),
  cancelJob: (id: string) => api<{ ok: true }>(`/api/jobs/${enc(id)}/cancel`, { body: {} }),

  memories: (q?: string) => api<T.Memory[]>(`/api/memory${q ? `?q=${enc(q)}` : ""}`),
  addMemory: (m: Required<T.MemoryInput>) => api<T.Memory>("/api/memory", { body: m }),
  updateMemory: (id: number, m: T.MemoryInput) => put<T.Memory>(`/api/memory/${id}`, m),
  deleteMemory: (id: number) => del(`/api/memory/${id}`),

  providers: () => api<T.ProviderView[]>("/api/providers"),
  presets: () => api<T.Preset[]>("/api/providers/presets"),
  addProvider: (p: T.ProviderConfig) => api<T.ProviderView>("/api/providers", { body: p }),
  updateProvider: (id: string, p: T.ProviderConfig) => put<T.ProviderView>(`/api/providers/${enc(id)}`, p),
  deleteProvider: (id: string) => del(`/api/providers/${enc(id)}`),
  activateProvider: (id: string) => api<{ ok: true }>(`/api/providers/${enc(id)}/activate`, { body: {} }),
  testProvider: (id: string) => api<T.TestReport>(`/api/providers/${enc(id)}/test`, { body: {} }),
  putSecret: (ref: string, value: string) => put<{ ok: true }>("/api/secrets", { ref, value }),

  destinations: () => api<T.Destination[]>("/api/destinations"),
  addDestination: (d: T.DestinationInput) => api<T.Destination>("/api/destinations", { body: d }),
  updateDestination: (id: string, d: T.DestinationInput) => put<T.Destination>(`/api/destinations/${enc(id)}`, d),
  deleteDestination: (id: string) => del(`/api/destinations/${enc(id)}`),
  testDestination: (id: string) => api<T.DestinationTest>(`/api/destinations/${enc(id)}/test`, { body: {} }),
  rcloneRuntime: () => api<T.RcloneRuntime>("/api/runtime/rclone"),
  installRclone: () => api<T.Job>("/api/runtime/rclone/install", { body: {} }),
  authorizeRclone: (b: T.RcloneAuthorizeInput) => api<{ token_ref: string }>("/api/destinations/rclone/authorize", { body: b }),
  pickFolder: () => api<{ path: string | null }>("/api/dialog/pick-folder", { body: {} }),
  archive: (q?: string) => api<T.ArchiveEntry[]>(`/api/archive${q ? `?q=${enc(q)}` : ""}`),
  restoreArchive: (id: number) => api<{ job_id: string }>(`/api/archive/${id}/restore`, { body: {} }),

  searchProviders: () => api<T.SearchProviderView[]>("/api/search/providers"),
  putSearchProvider: (kind: T.SearchKind, p: T.SearchProviderInput) => put<{ ok: true }>(`/api/search/providers/${kind}`, p),
  activateSearchProvider: (kind: T.SearchKind) => api<{ ok: true }>(`/api/search/providers/${kind}/activate`, { body: {} }),
  testSearchProvider: (kind: T.SearchKind) => api<T.SearchTest>(`/api/search/providers/${kind}/test`, { body: {} }),

  builtin: () => api<T.BuiltinView>("/api/builtin"),
  installBuiltin: (model_id: string) => api<T.Job>("/api/builtin/install", { body: { model_id } }),
  removeBuiltin: () => del<{ ok: true; freed_bytes: number }>("/api/builtin"),

  history: () => api<T.HistoryPoint[]>("/api/history"),
  settings: () => api<T.Settings>("/api/settings"),
  putSettings: (s: Partial<T.Settings>) => put<T.Settings>("/api/settings", s),
};

// EventSource cannot send headers; fetch streaming keeps the token out of the URL.
export function streamStatus(onEvent: (e: T.StatusEvent) => void, onState?: (connected: boolean) => void): () => void {
  let stopped = false;
  let ctl: AbortController | null = null;
  let delay = 500;

  const loop = async () => {
    while (!stopped) {
      ctl = new AbortController();
      try {
        const res = await fetch("/api/status/stream", {
          headers: { ...(token ? { Authorization: `Bearer ${token}` } : {}), Accept: "text/event-stream" },
          signal: ctl.signal,
        });
        if (!res.ok || !res.body) throw new ApiError(res.statusText, res.status);
        onState?.(true);
        delay = 500;
        await readSse(res.body, (data) => {
          try {
            onEvent(JSON.parse(data) as T.StatusEvent);
          } catch {
            // ignore malformed frame
          }
        });
      } catch {
        // fall through to reconnect
      }
      if (stopped) return;
      onState?.(false);
      await new Promise((r) => setTimeout(r, delay));
      delay = Math.min(delay * 2, 8000);
    }
  };
  void loop();
  return () => {
    stopped = true;
    ctl?.abort();
  };
}

export async function readSse(body: ReadableStream<Uint8Array>, onData: (data: string) => void): Promise<void> {
  const reader = body.getReader();
  const dec = new TextDecoder();
  let buf = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    buf += dec.decode(value, { stream: true });
    let i: number;
    while ((i = buf.search(/\r?\n\r?\n/)) >= 0) {
      const frame = buf.slice(0, i);
      buf = buf.slice(i).replace(/^\r?\n\r?\n/, "");
      const data = frame
        .split(/\r?\n/)
        .filter((l) => l.startsWith("data:"))
        .map((l) => l.slice(5).replace(/^ /, ""))
        .join("\n");
      if (data) onData(data);
    }
  }
}
