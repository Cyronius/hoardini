import { useCallback, useEffect, useState } from "react";
import { routes } from "../api/client";
import type { SearchKind, SearchProviderView, SearchTest } from "../api/types";
import { btnPrimary, btnSecondary, ErrorLine, errMsg, field, Field } from "./ui";

const KINDS: { kind: SearchKind; label: string; needsKey: boolean; needsUrl: boolean }[] = [
  { kind: "tavily", label: "Tavily", needsKey: true, needsUrl: false },
  { kind: "brave", label: "Brave Search", needsKey: true, needsUrl: false },
  { kind: "exa", label: "Exa", needsKey: true, needsUrl: false },
  { kind: "serper", label: "Serper", needsKey: true, needsUrl: false },
  { kind: "searxng", label: "SearXNG (self-hosted)", needsKey: false, needsUrl: true },
];

function Row({ meta, view, onChanged }: { meta: (typeof KINDS)[number]; view: SearchProviderView | undefined; onChanged: () => void }) {
  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [url, setUrl] = useState(view?.base_url ?? "");
  const [test, setTest] = useState<SearchTest | "pending" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    try {
      const body: { api_key?: string | null; base_url?: string | null } = {};
      if (key.trim()) {
        const ref = `secret://search/${meta.kind}/api_key`;
        await routes.putSecret(ref, key.trim());
        body.api_key = ref;
      }
      if (meta.needsUrl) body.base_url = url.trim() || null;
      await routes.putSearchProvider(meta.kind, body);
      if (body.api_key || (meta.needsUrl && url.trim())) await routes.activateSearchProvider(meta.kind);
      setKey("");
      setError(null);
      onChanged();
    } catch (e) {
      setError(errMsg(e));
    }
  };
  const runTest = async () => {
    setTest("pending");
    try {
      setTest(await routes.testSearchProvider(meta.kind));
    } catch (e) {
      setTest({ ok: false, error: errMsg(e) });
    }
  };

  const configured = view?.has_key || (meta.needsUrl && !!view?.base_url);
  return (
    <li className={`rounded-xl border p-2.5 ${view?.is_active ? "border-brass/50 bg-brass/5" : "border-line-soft bg-ink-850"}`}>
      <div className="flex items-center gap-2">
        <input
          type="radio"
          name="active-search"
          className="h-4 w-4 accent-[var(--color-brass)]"
          checked={!!view?.is_active}
          disabled={!configured}
          aria-label={`Use ${meta.label}`}
          onChange={() => void routes.activateSearchProvider(meta.kind).then(onChanged)}
        />
        <button className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => setOpen((o) => !o)}>
          <span className="font-bold">{meta.label}</span>
          <span className={`text-[length:var(--fs-xs)] ${configured ? "text-ok" : "text-faint"}`}>{configured ? "configured" : "not set up"}</span>
        </button>
        <button className={btnSecondary} disabled={!configured || test === "pending"} onClick={() => void runTest()}>
          {test === "pending" ? "Testing..." : "Test"}
        </button>
      </div>
      {test && test !== "pending" && (
        <div className={`mt-1.5 text-[length:var(--fs-sm)] ${test.ok ? "text-ok" : "text-err"}`}>
          {test.ok ? `Works${test.latency_ms != null ? ` · ${test.latency_ms} ms` : ""}.` : `Failed: ${test.error ?? "unknown error"}`}
        </div>
      )}
      {open && (
        <div className="mt-2 space-y-2">
          {meta.needsKey && (
            <Field label="API key" hint="Stored in the vault; never shown again.">
              <input className={field} type="password" autoComplete="new-password" placeholder={view?.has_key ? "Stored (leave blank to keep)" : ""} value={key} onChange={(e) => setKey(e.target.value)} />
            </Field>
          )}
          {meta.needsUrl && (
            <Field label="Instance URL">
              <input className={field} placeholder="http://127.0.0.1:8080" value={url} onChange={(e) => setUrl(e.target.value)} />
            </Field>
          )}
          <ErrorLine error={error} />
          <div className="flex justify-end">
            <button className={btnPrimary} onClick={() => void save()}>
              Save
            </button>
          </div>
        </div>
      )}
    </li>
  );
}

export function SearchTab() {
  const [rows, setRows] = useState<SearchProviderView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    routes.searchProviders().then(setRows).catch((e) => setError(errMsg(e)));
  }, []);
  useEffect(load, [load]);
  return (
    <div className="space-y-3">
      <p className="text-muted">Web search fills gaps the local evidence cannot. Queries are redacted first. Saving a key turns it on with that provider.</p>
      <ErrorLine error={error} />
      <ul className="space-y-2">
        {KINDS.map((m) => (
          <Row key={m.kind + (rows?.find((r) => r.kind === m.kind)?.has_key ? "k" : "")} meta={m} view={rows?.find((r) => r.kind === m.kind)} onChanged={load} />
        ))}
      </ul>
    </div>
  );
}
