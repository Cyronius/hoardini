import { useCallback, useEffect, useState } from "react";
import { routes } from "../api/client";
import type { Destination, DestinationInput, DestinationTest } from "../api/types";
import { humanBytes } from "../format";
import { EditIcon, FolderIcon, PlusIcon, TrashIcon } from "../components/icons";
import { IconButton } from "../components/IconButton";
import { btnDanger, btnPrimary, btnSecondary, ErrorLine, errMsg, field, Field } from "./ui";

type Draft = {
  id?: string;
  name: string;
  kind: Destination["kind"];
  path: string;
  unc: string;
  user: string;
  password: string;
  remote: string;
  remotePath: string;
};

const blank = (): Draft => ({ name: "", kind: "local", path: "", unc: "", user: "", password: "", remote: "", remotePath: "" });

function toDraft(d: Destination): Draft {
  const c = d.config as Record<string, string | undefined>;
  return {
    id: d.id,
    name: d.name,
    kind: d.kind,
    path: c.path ?? "",
    unc: c.unc ?? "",
    user: c.user ?? "",
    password: "",
    remote: c.remote ?? "",
    remotePath: c.path ?? "",
  };
}

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "dest";

function DestForm({ draft, setDraft, onSave, onCancel, error }: { draft: Draft; setDraft: (d: Draft) => void; onSave: () => void; onCancel: () => void; error: string | null }) {
  const [pickErr, setPickErr] = useState<string | null>(null);
  const pick = async () => {
    try {
      const r = await routes.pickFolder();
      if (r.path) setDraft({ ...draft, path: r.path });
      setPickErr(null);
    } catch (e) {
      setPickErr(errMsg(e).includes("501") || /not implemented/i.test(errMsg(e)) ? "No folder dialog in headless mode; type the path." : errMsg(e));
    }
  };
  const valid = draft.name.trim() && (draft.kind === "local" ? draft.path.trim() : draft.kind === "smb" ? draft.unc.trim().startsWith("\\\\") : draft.remote.trim());
  return (
    <div className="space-y-2 rounded-xl border border-brass/40 bg-ink-850 p-3">
      <div className="grid grid-cols-2 gap-2">
        <Field label="Name">
          <input className={field} value={draft.name} placeholder="NAS" onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
        </Field>
        <Field label="Type">
          <select className={field} value={draft.kind} onChange={(e) => setDraft({ ...draft, kind: e.target.value as Destination["kind"] })}>
            <option value="local">Local folder</option>
            <option value="smb">Network share (SMB)</option>
            <option value="rclone">Cloud (rclone)</option>
          </select>
        </Field>
      </div>
      {draft.kind === "local" && (
        <Field label="Folder">
          <div className="flex gap-2">
            <input className={field} value={draft.path} placeholder="D:\Archive" onChange={(e) => setDraft({ ...draft, path: e.target.value })} />
            <button className={`${btnSecondary} flex shrink-0 items-center gap-1.5`} onClick={() => void pick()}>
              <FolderIcon /> Pick folder
            </button>
          </div>
          {pickErr && <div className="mt-1 text-[length:var(--fs-xs)] text-err">{pickErr}</div>}
        </Field>
      )}
      {draft.kind === "smb" && (
        <>
          <Field label="Share path" hint="UNC path, for example \\nas\backups\hoardini">
            <input className={field} value={draft.unc} placeholder="\\nas\share" onChange={(e) => setDraft({ ...draft, unc: e.target.value })} />
          </Field>
          <div className="grid grid-cols-2 gap-2">
            <Field label="User (optional)">
              <input className={field} value={draft.user} autoComplete="off" onChange={(e) => setDraft({ ...draft, user: e.target.value })} />
            </Field>
            <Field label="Password (optional)" hint={draft.id ? "Leave blank to keep the stored one." : undefined}>
              <input className={field} type="password" value={draft.password} autoComplete="new-password" onChange={(e) => setDraft({ ...draft, password: e.target.value })} />
            </Field>
          </div>
        </>
      )}
      {draft.kind === "rclone" && (
        <>
          <p className="rounded-lg bg-ink-800 px-2.5 py-1.5 text-[length:var(--fs-sm)] text-muted">
            Cloud transfers use rclone, which is not wired up yet. You can save the remote name now.
          </p>
          <div className="grid grid-cols-2 gap-2">
            <Field label="Remote">
              <input className={field} value={draft.remote} placeholder="gdrive" onChange={(e) => setDraft({ ...draft, remote: e.target.value })} />
            </Field>
            <Field label="Path">
              <input className={field} value={draft.remotePath} placeholder="hoardini" onChange={(e) => setDraft({ ...draft, remotePath: e.target.value })} />
            </Field>
          </div>
        </>
      )}
      <ErrorLine error={error} />
      <div className="flex justify-end gap-2">
        <button className={btnSecondary} onClick={onCancel}>
          Cancel
        </button>
        <button className={btnPrimary} disabled={!valid} onClick={onSave}>
          Save
        </button>
      </div>
    </div>
  );
}

export function DestinationsTab() {
  const [rows, setRows] = useState<Destination[] | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [tests, setTests] = useState<Record<string, DestinationTest | "pending">>({});
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    routes.destinations().then(setRows).catch((e) => setError(errMsg(e)));
  }, []);
  useEffect(load, [load]);

  const save = async () => {
    if (!draft) return;
    try {
      const config: Record<string, unknown> =
        draft.kind === "local"
          ? { path: draft.path.trim() }
          : draft.kind === "smb"
            ? { unc: draft.unc.trim(), user: draft.user.trim() || undefined }
            : { remote: draft.remote.trim(), path: draft.remotePath.trim() };
      if (draft.kind === "smb" && draft.password) {
        const ref = `secret://destination/${slug(draft.name)}/password`;
        await routes.putSecret(ref, draft.password);
        config.password_ref = ref;
      }
      const body: DestinationInput = { name: draft.name.trim(), kind: draft.kind, config };
      if (draft.id) await routes.updateDestination(draft.id, body);
      else await routes.addDestination(body);
      setDraft(null);
      setError(null);
      load();
    } catch (e) {
      setError(errMsg(e));
    }
  };

  const test = async (id: string) => {
    setTests((t) => ({ ...t, [id]: "pending" }));
    try {
      const r = await routes.testDestination(id);
      setTests((t) => ({ ...t, [id]: r }));
    } catch (e) {
      setTests((t) => ({ ...t, [id]: { ok: false, error: errMsg(e) } }));
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <p className="text-muted">Where "archive, then delete" can send things.</p>
        <IconButton title="Add destination" aria-label="Add destination" className="text-brass" onClick={() => setDraft(blank())}>
          <PlusIcon />
        </IconButton>
      </div>
      <ErrorLine error={draft ? null : error} />
      {draft && !draft.id && <DestForm draft={draft} setDraft={setDraft} onSave={() => void save()} onCancel={() => setDraft(null)} error={error} />}
      {rows?.length === 0 && !draft && <div className="text-muted">No destinations yet. Without one, you can still delete, but not archive.</div>}
      <ul className="space-y-2">
        {rows?.map((d) => {
          if (draft?.id === d.id) {
            return (
              <li key={d.id}>
                <DestForm draft={draft} setDraft={setDraft} onSave={() => void save()} onCancel={() => setDraft(null)} error={error} />
              </li>
            );
          }
          const t = tests[d.id];
          const where = (d.config.path ?? d.config.unc ?? d.config.remote ?? "") as string;
          return (
            <li key={d.id} className="rounded-xl border border-line-soft bg-ink-850 p-2.5">
              <div className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="font-bold">{d.name}</span>
                    <span className="rounded-full bg-ink-800 px-2 py-px text-[length:var(--fs-xs)] text-muted">{d.kind}</span>
                  </div>
                  <div className="truncate font-mono text-[length:var(--fs-xs)] text-faint" title={where}>
                    {where}
                  </div>
                </div>
                {confirmId === d.id ? (
                  <div className="flex gap-1">
                    <button
                      className={btnDanger}
                      onClick={() => {
                        void routes.deleteDestination(d.id).then(() => {
                          setConfirmId(null);
                          load();
                        });
                      }}
                    >
                      Delete
                    </button>
                    <button className={btnSecondary} onClick={() => setConfirmId(null)}>
                      Keep
                    </button>
                  </div>
                ) : (
                  <>
                    <button className={btnSecondary} disabled={t === "pending"} onClick={() => void test(d.id)}>
                      {t === "pending" ? "Testing..." : "Test"}
                    </button>
                    <IconButton size="sm" title="Edit destination" aria-label="Edit destination" className="text-faint hover:text-paper" onClick={() => setDraft(toDraft(d))}>
                      <EditIcon />
                    </IconButton>
                    <IconButton size="sm" title="Delete destination" aria-label="Delete destination" className="text-faint hover:text-err" onClick={() => setConfirmId(d.id)}>
                      <TrashIcon />
                    </IconButton>
                  </>
                )}
              </div>
              {t && t !== "pending" && (
                <div className={`mt-1.5 text-[length:var(--fs-sm)] ${t.ok ? "text-ok" : "text-err"}`}>
                  {t.ok ? `Reachable${t.free_bytes != null ? `, ${humanBytes(t.free_bytes)} free` : ""}.` : `Failed: ${t.error ?? "unknown error"}`}
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
