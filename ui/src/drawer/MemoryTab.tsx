import { useCallback, useEffect, useState } from "react";
import { routes } from "../api/client";
import type { Memory } from "../api/types";
import { humanAge } from "../format";
import { EditIcon, PlusIcon, TrashIcon } from "../components/icons";
import { IconButton } from "../components/IconButton";
import { btnDanger, btnPrimary, btnSecondary, ErrorLine, errMsg, field, Field } from "./ui";

const SCOPES: Memory["scope"][] = ["path", "glob", "global"];
const KINDS: Memory["kind"][] = ["fact", "preference", "protect", "verdict"];

type Draft = { id?: number; scope: Memory["scope"]; key: string; kind: Memory["kind"]; text: string };

function urlOf(source: string): string | null {
  return /https?:\/\/\S+/.exec(source)?.[0] ?? null;
}

function MemoryEditor({ draft, onChange, onSave, onCancel }: { draft: Draft; onChange: (d: Draft) => void; onSave: () => void; onCancel: () => void }) {
  return (
    <div className="space-y-2 rounded-xl border border-brass/40 bg-ink-850 p-3">
      <div className="grid grid-cols-2 gap-2">
        <Field label="Scope">
          <select className={field} value={draft.scope} onChange={(e) => onChange({ ...draft, scope: e.target.value as Memory["scope"] })}>
            {SCOPES.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
        </Field>
        <Field label="Kind">
          <select className={field} value={draft.kind} onChange={(e) => onChange({ ...draft, kind: e.target.value as Memory["kind"] })}>
            {KINDS.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
        </Field>
      </div>
      <Field label="Key" hint={draft.scope === "global" ? "Global notes need no path." : "A path like ~\\Downloads or a glob like **\\node_modules"}>
        <input className={field} value={draft.key} disabled={draft.scope === "global"} onChange={(e) => onChange({ ...draft, key: e.target.value })} />
      </Field>
      <Field label="Note">
        <textarea className={`${field} min-h-[4rem] resize-y`} value={draft.text} onChange={(e) => onChange({ ...draft, text: e.target.value })} />
      </Field>
      <div className="flex justify-end gap-2">
        <button className={btnSecondary} onClick={onCancel}>
          Cancel
        </button>
        <button className={btnPrimary} disabled={!draft.text.trim()} onClick={onSave}>
          Save
        </button>
      </div>
    </div>
  );
}

export function MemoryTab() {
  const [rows, setRows] = useState<Memory[] | null>(null);
  const [q, setQ] = useState("");
  const [draft, setDraft] = useState<Draft | null>(null);
  const [confirmId, setConfirmId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    routes
      .memories(q.trim() || undefined)
      .then(setRows)
      .catch((e) => setError(errMsg(e)));
  }, [q]);
  useEffect(() => {
    const t = setTimeout(load, 200);
    return () => clearTimeout(t);
  }, [load]);

  const save = async () => {
    if (!draft) return;
    try {
      if (draft.id != null) await routes.updateMemory(draft.id, { scope: draft.scope, key: draft.key, kind: draft.kind, text: draft.text });
      else await routes.addMemory({ scope: draft.scope, key: draft.scope === "global" ? "" : draft.key, kind: draft.kind, text: draft.text });
      setDraft(null);
      setError(null);
      load();
    } catch (e) {
      setError(errMsg(e));
    }
  };
  const remove = async (id: number) => {
    try {
      await routes.deleteMemory(id);
      setConfirmId(null);
      load();
    } catch (e) {
      setError(errMsg(e));
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <input className={field} placeholder="Search notes..." value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search memory" />
        <IconButton title="Add note" aria-label="Add note" className="text-brass" onClick={() => setDraft({ scope: "path", key: "", kind: "fact", text: "" })}>
          <PlusIcon />
        </IconButton>
      </div>
      <ErrorLine error={error} />
      {draft && draft.id == null && <MemoryEditor draft={draft} onChange={setDraft} onSave={() => void save()} onCancel={() => setDraft(null)} />}
      {rows == null && <div className="text-muted pulse-soft">Loading...</div>}
      {rows?.length === 0 && <div className="text-muted">{q ? "No notes match." : "Nothing remembered yet. Tell me a preference and I'll note it."}</div>}
      <ul className="space-y-2">
        {rows?.map((m) => {
          if (draft?.id === m.id) {
            return (
              <li key={m.id}>
                <MemoryEditor draft={draft} onChange={setDraft} onSave={() => void save()} onCancel={() => setDraft(null)} />
              </li>
            );
          }
          const url = urlOf(m.source);
          return (
            <li key={m.id} className="rounded-xl border border-line-soft bg-ink-850 p-2.5">
              <div className="flex items-start gap-2">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-1.5 text-[length:var(--fs-xs)]">
                    <span className="rounded-full bg-ink-800 px-2 py-px font-bold text-paper-dim">{m.kind}</span>
                    <span className="rounded-full bg-ink-800 px-2 py-px text-muted">{m.scope}</span>
                    {m.key && (
                      <span className="min-w-0 truncate font-mono text-muted" title={m.key}>
                        {m.key}
                      </span>
                    )}
                  </div>
                  <p className="mt-1 break-words">{m.text}</p>
                  <div className="mt-1 text-[length:var(--fs-xs)] text-faint">
                    {url ? (
                      <>
                        from the web:{" "}
                        <a className="break-all underline hover:text-brass" href={url} target="_blank" rel="noopener noreferrer">
                          {url}
                        </a>
                      </>
                    ) : (
                      <>from {m.source}</>
                    )}
                    {" · "}updated {humanAge(Date.now() - m.updated_ms)}
                  </div>
                </div>
                {confirmId === m.id ? (
                  <div className="flex shrink-0 gap-1">
                    <button className={btnDanger} onClick={() => void remove(m.id)}>
                      Delete
                    </button>
                    <button className={btnSecondary} onClick={() => setConfirmId(null)}>
                      Keep
                    </button>
                  </div>
                ) : (
                  <div className="flex shrink-0">
                    <IconButton size="sm" title="Edit note" aria-label="Edit note" className="text-faint hover:text-paper" onClick={() => setDraft({ id: m.id, scope: m.scope, key: m.key, kind: m.kind, text: m.text })}>
                      <EditIcon />
                    </IconButton>
                    <IconButton size="sm" title="Delete note" aria-label="Delete note" className="text-faint hover:text-err" onClick={() => setConfirmId(m.id)}>
                      <TrashIcon />
                    </IconButton>
                  </div>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
