import { useEffect, useState } from "react";
import { routes } from "../api/client";
import type { Settings } from "../api/types";
import { humanBytes } from "../format";
import { useStore } from "../store";
import { SavedTick, useAutoSave } from "./autosave";
import { ErrorLine, errMsg, Section, Toggle } from "./ui";

// Each reply is the whole settings, so writes go one at a time and the last reply is the last write.
let writes: Promise<unknown> = Promise.resolve();
const putInTurn = (patch: Parameters<typeof routes.putSettings>[0]) => {
  const run = writes.then(() => routes.putSettings(patch));
  writes = run.catch(() => {});
  return run;
};

// The header's Look again rebuilds the review; this one rescans the drive itself first.
function Rescan() {
  const refreshing = useStore((st) => st.status?.index.state === "refreshing");
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="mt-3 flex items-center gap-3">
      <button
        type="button"
        className="rounded-lg border border-line px-3 py-1 text-[length:var(--fs-sm)] font-semibold text-paper-dim hover:text-paper disabled:opacity-50"
        disabled={refreshing}
        onClick={() => {
          setError(null);
          routes.refresh().catch((e) => setError(errMsg(e)));
        }}
      >
        {refreshing ? "Rescanning…" : "Rescan now"}
      </button>
      <span className="text-[length:var(--fs-xs)] text-faint">Hoardini keeps up with changes on its own while it is open.</span>
      <ErrorLine error={error} />
    </div>
  );
}

export function SettingsTab() {
  const [s, setS] = useState<Settings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const online = useAutoSave(async (on: boolean) => {
    try {
      setS(await putInTurn({ online_checks: { asked: true, services: { huggingface: on, winget: on } } }));
    } catch (e) {
      routes.settings().then(setS).catch(() => {});
      throw e;
    }
  });

  useEffect(() => {
    routes
      .settings()
      .then(setS)
      .catch((e) => setLoadError(errMsg(e)));
  }, []);

  if (!s) return <div className="text-muted">{loadError ? <ErrorLine error={loadError} /> : <span className="pulse-soft">Loading...</span>}</div>;

  const anyOnline = Object.values(s.online_checks.services).some(Boolean);

  return (
    <div className="space-y-5">
      <Section title="Index">
        <div className="flex items-baseline gap-2 text-[length:var(--fs-sm)]">
          <span className="text-faint">Index size</span>
          <span className="font-bold tabular-nums" title="Read-only">
            {humanBytes(s.index_size_bytes)}
          </span>
        </div>
        <Rescan />
      </Section>
      <Section title="Online checks" action={<SavedTick status={online.status} />}>
        <Toggle
          label="Look up big files online"
          hint="Confirms that a big file can be downloaded again before it is offered as re-downloadable: model files against their public repo (such as Hugging Face), installers against winget. Sends file names and package or repo names, never file contents or hashes. Checks against your own copies, git, and cloud sync always run on this PC."
          checked={anyOnline}
          onChange={(on) => {
            setS({ ...s, online_checks: { asked: true, services: { huggingface: on, winget: on } } });
            online.commit(on);
          }}
        />
        <ErrorLine error={online.error} />
      </Section>
    </div>
  );
}
