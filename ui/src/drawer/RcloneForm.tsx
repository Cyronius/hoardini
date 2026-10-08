import { useCallback, useEffect, useState } from "react";
import { routes } from "../api/client";
import type { Job, RcloneRuntime } from "../api/types";
import { humanBytes } from "../format";
import { btnPrimary, btnSecondary, ErrorLine, errMsg, field, Field } from "./ui";

type RcField = { key: string; label: string; secret?: boolean; placeholder?: string; hint?: string; optional?: boolean };
type RcPreset = {
  id: string;
  type: string;
  label: string;
  oauth?: boolean;
  fields: RcField[];
  fixed?: Record<string, string>;
  note?: string;
  docs?: { label: string; href: string };
};

const OWN_CLIENT: RcField[] = [
  { key: "client_id", label: "OAuth client ID", secret: true },
  { key: "client_secret", label: "OAuth client secret", secret: true },
];
const OPTIONAL_CLIENT: RcField[] = OWN_CLIENT.map((f) => ({ ...f, optional: true }));

export const PRESETS: RcPreset[] = [
  {
    id: "drive",
    type: "drive",
    label: "Google Drive",
    oauth: true,
    fields: OWN_CLIENT,
    fixed: { scope: "drive.file" },
    note: "Google Drive needs your own OAuth client ID. Hoardini only asks for the drive.file scope, so it can see just the files it creates.",
    docs: { label: "How to make a client ID", href: "https://rclone.org/drive/#making-your-own-client-id" },
  },
  { id: "dropbox", type: "dropbox", label: "Dropbox", oauth: true, fields: OPTIONAL_CLIENT, note: "Leave the client fields empty to use rclone's built-in app." },
  { id: "onedrive", type: "onedrive", label: "OneDrive", oauth: true, fields: OPTIONAL_CLIENT, note: "Leave the client fields empty to use rclone's built-in app." },
  {
    id: "s3",
    type: "s3",
    label: "Amazon S3 / S3-compatible",
    fields: [
      { key: "provider", label: "Provider", placeholder: "AWS", optional: true },
      { key: "access_key_id", label: "Access key ID", secret: true },
      { key: "secret_access_key", label: "Secret access key", secret: true },
      { key: "region", label: "Region", placeholder: "us-east-1", optional: true },
      { key: "endpoint", label: "Endpoint", placeholder: "only for non-AWS", optional: true },
    ],
  },
  {
    id: "b2",
    type: "b2",
    label: "Backblaze B2",
    fields: [
      { key: "account", label: "Key ID", secret: true },
      { key: "key", label: "Application key", secret: true },
    ],
  },
  {
    id: "r2",
    type: "s3",
    label: "Cloudflare R2",
    fixed: { provider: "Cloudflare" },
    fields: [
      { key: "access_key_id", label: "Access key ID", secret: true },
      { key: "secret_access_key", label: "Secret access key", secret: true },
      { key: "endpoint", label: "Endpoint", placeholder: "https://<account>.r2.cloudflarestorage.com" },
    ],
  },
  {
    id: "sftp",
    type: "sftp",
    label: "SFTP",
    fields: [
      { key: "host", label: "Host" },
      { key: "user", label: "User" },
      { key: "port", label: "Port", placeholder: "22", optional: true },
      { key: "pass", label: "Password", secret: true },
    ],
  },
  {
    id: "smb",
    type: "smb",
    label: "SMB (via rclone)",
    fields: [
      { key: "host", label: "Host" },
      { key: "user", label: "User" },
      { key: "pass", label: "Password", secret: true },
    ],
  },
];

export type RcDraft = {
  preset: string;
  params: Record<string, string>;
  // Newly typed secret values, keyed by field; blank keeps the stored reference.
  secrets: Record<string, string>;
  tokenRef: string;
};

export const blankRc = (): RcDraft => ({ preset: "drive", params: {}, secrets: {}, tokenRef: "" });

export function rcFromConfig(config: Record<string, unknown>): RcDraft {
  const params = { ...((config.params as Record<string, string> | undefined) ?? {}) };
  const type = String(config.type ?? "");
  const preset = type === "s3" && params.provider === "Cloudflare" ? "r2" : (PRESETS.find((p) => p.type === type)?.id ?? "drive");
  const tokenRef = params.token ?? "";
  delete params.token;
  return { preset, params, secrets: {}, tokenRef };
}

const presetOf = (id: string) => PRESETS.find((p) => p.id === id) ?? PRESETS[0]!;
const isRef = (v: string | undefined) => !!v && v.startsWith("secret://");

function haveSecret(rc: RcDraft, key: string) {
  return !!rc.secrets[key]?.trim() || isRef(rc.params[key]);
}

export function rcValid(rc: RcDraft): boolean {
  const p = presetOf(rc.preset);
  const fieldsOk = p.fields.every((f) => f.optional || (f.secret ? haveSecret(rc, f.key) : !!rc.params[f.key]?.trim()));
  return fieldsOk && (!p.oauth || !!rc.tokenRef);
}

const refFor = (slug: string, key: string) => `secret://destination/${slug}/${key}`;

// Typed secrets go to the vault; the saved config only carries references.
async function storeSecrets(rc: RcDraft, slug: string): Promise<Record<string, string>> {
  const params = { ...rc.params };
  for (const f of presetOf(rc.preset).fields) {
    const typed = rc.secrets[f.key]?.trim();
    if (f.secret && typed) {
      const ref = refFor(slug, f.key);
      await routes.putSecret(ref, typed);
      params[f.key] = ref;
    }
  }
  return params;
}

export async function rcloneConfig(rc: RcDraft, remote: string, path: string, slug: string): Promise<Record<string, unknown>> {
  const p = presetOf(rc.preset);
  const params: Record<string, string> = { ...(await storeSecrets(rc, slug)), ...p.fixed };
  if (p.oauth && rc.tokenRef) params.token = rc.tokenRef;
  for (const k of Object.keys(params)) if (!params[k]) delete params[k];
  return { remote, type: p.type, path, params };
}

function RuntimePanel() {
  const [rt, setRt] = useState<RcloneRuntime | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    routes.rcloneRuntime().then(setRt).catch((e) => setError(errMsg(e)));
  }, []);
  useEffect(load, [load]);

  useEffect(() => {
    if (!job || job.status !== "running") return;
    const t = setInterval(() => {
      routes
        .jobs()
        .then((all) => {
          const j = all.find((x) => x.id === job.id);
          if (!j) return;
          setJob(j);
          if (j.status !== "running") {
            if (j.status === "failed") setError(j.error ?? "The download failed.");
            load();
          }
        })
        .catch((e) => setError(errMsg(e)));
    }, 500);
    return () => clearInterval(t);
  }, [job, load]);

  const install = async () => {
    setError(null);
    try {
      setJob(await routes.installRclone());
    } catch (e) {
      setError(errMsg(e));
    }
  };

  if (!rt) return <ErrorLine error={error} />;
  const running = job?.status === "running";
  const pct = job && job.progress.total_bytes > 0 ? Math.min(100, (100 * job.progress.done_bytes) / job.progress.total_bytes) : 0;
  return (
    <div className="space-y-1.5 rounded-lg bg-ink-800 px-2.5 py-2 text-[length:var(--fs-sm)]">
      {rt.installed ? (
        <div className="text-muted">rclone {rt.version} is installed.</div>
      ) : (
        <>
          <div className="flex items-center justify-between gap-2">
            <span className="text-muted">
              Cloud transfers use rclone {rt.version} ({humanBytes(rt.size)}), downloaded from rclone.org and checked against a pinned SHA-256.
            </span>
            <button className={btnSecondary} disabled={running} onClick={() => void install()}>
              {running ? "Downloading..." : "Install"}
            </button>
          </div>
          {running && (
            <div className="h-1.5 overflow-hidden rounded-full bg-ink-900" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
              <div className="h-full bg-brass transition-[width]" style={{ width: `${pct}%` }} />
            </div>
          )}
        </>
      )}
      <ErrorLine error={error} />
    </div>
  );
}

type Props = {
  rc: RcDraft;
  setRc: (rc: RcDraft) => void;
  remote: string;
  setRemote: (v: string) => void;
  path: string;
  setPath: (v: string) => void;
  slug: string;
};

export function RcloneFields({ rc, setRc, remote, setRemote, path, setPath, slug }: Props) {
  const p = presetOf(rc.preset);
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);

  const setField = (key: string, secret: boolean | undefined, value: string) =>
    setRc(secret ? { ...rc, secrets: { ...rc.secrets, [key]: value } } : { ...rc, params: { ...rc.params, [key]: value } });

  const authorize = async () => {
    setAuthBusy(true);
    setAuthError(null);
    try {
      const params = await storeSecrets(rc, slug);
      const next = { ...rc, params, secrets: {} };
      const res = await routes.authorizeRclone({ type: p.type, client_id_ref: params.client_id, client_secret_ref: params.client_secret });
      setRc({ ...next, tokenRef: res.token_ref });
    } catch (e) {
      setAuthError(errMsg(e));
    } finally {
      setAuthBusy(false);
    }
  };

  const clientReady = !p.fields.some((f) => f.secret && !f.optional && !haveSecret(rc, f.key));
  return (
    <div className="space-y-2">
      <RuntimePanel />
      <div className="grid grid-cols-2 gap-2">
        <Field label="Service">
          <select className={field} value={rc.preset} onChange={(e) => setRc({ ...blankRc(), preset: e.target.value })}>
            {PRESETS.map((x) => (
              <option key={x.id} value={x.id}>
                {x.label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Remote name" hint="Letters, digits, - and _ only.">
          <input className={field} value={remote} placeholder={slug || "gdrive"} onChange={(e) => setRemote(e.target.value)} />
        </Field>
      </div>
      {p.note && (
        <p className="text-[length:var(--fs-xs)] text-faint">
          {p.note}{" "}
          {p.docs && (
            <a className="text-brass underline" href={p.docs.href} target="_blank" rel="noreferrer">
              {p.docs.label}
            </a>
          )}
        </p>
      )}
      <div className="grid grid-cols-2 gap-2">
        {p.fields.map((f) => (
          <Field key={f.key} label={f.label + (f.optional ? " (optional)" : "")} hint={f.secret && isRef(rc.params[f.key]) ? "Stored in the vault. Leave blank to keep it." : f.hint}>
            <input
              className={field}
              type={f.secret ? "password" : "text"}
              autoComplete="off"
              placeholder={f.placeholder}
              value={f.secret ? (rc.secrets[f.key] ?? "") : (rc.params[f.key] ?? "")}
              onChange={(e) => setField(f.key, f.secret, e.target.value)}
            />
          </Field>
        ))}
      </div>
      <Field label="Folder" hint="Where archives go inside the remote.">
        <input className={field} value={path} placeholder="hoardini" onChange={(e) => setPath(e.target.value)} />
      </Field>
      {p.oauth && (
        <div className="flex items-center gap-2">
          <button className={btnPrimary} disabled={authBusy || !clientReady} onClick={() => void authorize()}>
            {authBusy ? "Waiting for the browser..." : rc.tokenRef ? "Authorize again" : "Authorize"}
          </button>
          <span className={`text-[length:var(--fs-sm)] ${rc.tokenRef ? "text-ok" : "text-muted"}`}>
            {rc.tokenRef ? "Signed in." : "Opens your browser to sign in."}
          </span>
        </div>
      )}
      <ErrorLine error={authError} />
    </div>
  );
}
