import { useEffect, useRef, useState, type ReactNode } from "react";
import { routes } from "../api/client";
import type { ActKind, ActResult, Children, Destination, Job, ReviewArea, TypeRow } from "../api/types";
import { fileName, humanBytes, shortDate } from "../format";
import { useReview } from "../review/state";
import { useStore } from "../store";
import { SumoSvg } from "../components/Mascot";
import { ExplorerIcon } from "../components/icons";
import { Ask, type AskHandle } from "./Ask";
import { choosable, picked, worth, type CallRow, type Part } from "./groups";
import { HoldPill } from "./HoldPill";
import { approveParts, toggleTick } from "./ticks";

const ENDED: Job["status"][] = ["done", "failed", "cancelled"];
const VERB: Record<ActKind, [string, string]> = {
  delete: ["Deleting", "Deleted"],
  compress: ["Shrinking", "Shrunk"],
  online_only: ["Freeing", "Freed"],
  archive: ["Moving", "Moved"],
};
const CANT: Record<ActKind, string> = { delete: "delete", compress: "shrink", online_only: "free", archive: "move" };

// pid 4 is Windows itself, which holds a virtual disk open while WSL or Docker runs.
export function explainBlock(reason: string): { say: string } {
  const m = /^Open in (.+?) \(pid (\d+)\)/.exec(reason);
  if (!m) return { say: reason };
  const [, app, pid] = m;
  if (pid === "4" || app === "System") return { say: "Windows has a file in here open. WSL or Docker is probably running. Shut it down (wsl --shutdown), then try again." };
  return { say: `${app} has a file in here open. Close it, then try again.` };
}

const FIND_OUT ="What is this? Look inside it, and search the web if that helps. Then tell me whether to keep it, delete it, or move it elsewhere.";

type Target = { path: string; name: string; size: number };
type Outcome = { kind: ActKind; result: ActResult | null; error: string | null };

function Bubble({ children }: { children: ReactNode }) {
  return (
    <div className="grid grid-cols-[44px_minmax(0,1fr)] items-start gap-2.5">
      <SumoSvg act="idle" className="h-11 w-11" />
      <div className="rounded-[4px_14px_14px_14px] bg-ink-850 px-3 py-2 text-[length:var(--fs-sm)] [overflow-wrap:anywhere]">{children}</div>
    </div>
  );
}

function typeLine(rows: TypeRow[], total: number): string | null {
  const top = rows.filter((r) => r.ext).slice(0, 3);
  if (!top.length || total <= 0) return null;
  return top
    .map((r) => `${Math.round((r.bytes / total) * 100)}% .${r.ext}${r.label ? ` (${r.label.replace(/\.$/, "")})` : ""}`)
    .join(", ");
}

function RevealButton({ path, label }: { path: string; label?: boolean }) {
  const open = () => void routes.reveal(path).catch(() => {});
  return label ? (
    <button type="button" className="inline-flex items-center gap-1.5 rounded-full bg-brass px-3 py-1 text-[length:var(--fs-sm)] font-extrabold text-on-accent hover:brightness-110" onClick={open}>
      <ExplorerIcon size={16} />
      Show in Explorer
    </button>
  ) : (
    <button type="button" className="rounded-md bg-brass/15 p-1.5 text-brass hover:bg-brass hover:text-on-accent" title="Show in Explorer" aria-label="Show in Explorer" onClick={open}>
      <ExplorerIcon size={16} />
    </button>
  );
}

function Inside({ path, size, onOpen }: { path: string; size: number; onOpen: (t: Target) => void }) {
  const [kids, setKids] = useState<Children | null>(null);
  useEffect(() => {
    let live = true;
    routes
      .children(path, 6)
      .then((c) => live && setKids(c))
      .catch(() => live && setKids(null));
    return () => {
      live = false;
    };
  }, [path]);
  const rows = kids?.children.filter((c) => c.size > 0) ?? [];
  if (!rows.length) return null;
  const total = Math.max(size, 1);
  return (
    <div className="grid gap-2">
      <div className="flex h-2.5 gap-0.5 overflow-hidden rounded-full bg-ink-800" aria-hidden="true">
        {rows.map((c, i) => (
          <span key={c.path} className="h-full bg-brass" style={{ width: `${(c.size / total) * 100}%`, opacity: 1 - i * 0.14 }} />
        ))}
      </div>
      <ul className="grid">
        {rows.map((c) => (
          <li key={c.path} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-t border-line-soft">
            <button
              type="button"
              className="grid w-full grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-2.5 py-1.5 text-left text-[length:var(--fs-sm)] hover:text-brass disabled:hover:text-inherit"
              disabled={c.kind !== "dir"}
              onClick={() => onOpen({ path: c.path, name: c.name, size: c.size })}
              title={c.kind === "dir" ? `Look inside ${c.name}` : c.path}
            >
              <span className="truncate">{c.name}</span>
              <span className="tabular-nums text-faint">{Math.round((c.size / total) * 100)}%</span>
              <span className="tabular-nums font-bold text-paper-dim">{humanBytes(c.size)}</span>
            </button>
            <RevealButton path={c.path} />
          </li>
        ))}
      </ul>
    </div>
  );
}

function useJob(id: string | null | undefined): Job | undefined {
  return useStore((s) => (id ? s.jobs[id] : undefined));
}

function Result({ o, onDone }: { o: Outcome; onDone: () => void }) {
  const job = useJob(o.result?.job_id);
  const [verb, past] = VERB[o.kind];
  const ended = job && ENDED.includes(job.status);
  useEffect(() => {
    if (job?.status !== "done") return;
    const t = setTimeout(onDone, 1600);
    return () => clearTimeout(t);
  }, [job?.status, onDone]);
  if (o.error) return <p className="text-[length:var(--fs-sm)] text-err">{o.error}</p>;
  const blocked = o.result?.blocked ?? [];
  return (
    <div className="grid gap-1 text-[length:var(--fs-sm)]" role="status">
      {o.result?.job_id && !ended && (
        <p className="font-bold text-paper-dim">
          {verb}… {job && job.progress.total_bytes > 0 ? `${Math.round((job.progress.done_bytes / job.progress.total_bytes) * 100)}%` : ""}
        </p>
      )}
      {job?.status === "done" && <p className="font-extrabold">{o.kind === "delete" || o.kind === "online_only" ? `${past}. Freed ${humanBytes(job.freed_bytes)}.` : `${past}.`}</p>}
      {job?.status === "failed" && <p className="text-err">{job.error ?? "It didn't work."}</p>}
      {blocked.length > 0 && (
        <div role="alert" className="grid gap-1 rounded-xl border border-err/50 bg-err/10 px-3 py-2">
          <p className="font-extrabold text-err">{o.result?.job_id ? "Some of it stayed." : `Couldn't ${CANT[o.kind]} this.`}</p>
          <ul className="grid gap-1.5">
            {blocked.slice(0, 4).map((b) => {
              const why = explainBlock(b.reason);
              return (
                <li key={b.path}>
                  <p className="text-paper">{why.say}</p>
                  {why.say !== b.reason && <p className="text-[length:var(--fs-xs)] text-faint">{b.reason}</p>}
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}

function PartBody({ part, onDone }: { part: Part; onDone: () => void }) {
  const live = useStore((s) => s.proposals[part.proposal.id]) ?? part.proposal;
  const [jobs, setJobs] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const job = useJob(jobs?.[0]);
  const rows = live.items.filter((i) => i.status === "pending" || jobs);
  const bytes = picked(live).reduce((a, i) => a + worth(i), 0);
  const dest = live.destination?.name;
  const go = async () => {
    const r = await approveParts([{ ...part, proposal: live }]);
    if (r.errors.length) setError(r.errors[0]!);
    setJobs(r.jobs);
  };
  useEffect(() => {
    if (job?.status !== "done") return;
    const t = setTimeout(onDone, 1600);
    return () => clearTimeout(t);
  }, [job?.status, onDone]);
  return (
    <>
      <ul className="grid">
        {rows.map((i) => (
          <li key={i.id} className="grid grid-cols-[20px_minmax(0,1fr)_auto] items-start gap-x-2.5 border-t border-line-soft py-1.5 text-[length:var(--fs-sm)]">
            {choosable(i) && !jobs ? (
              <input type="checkbox" className="mt-0.5 h-4 w-4 accent-[var(--color-brass)]" checked={i.selected} aria-label={`Include ${fileName(i.path)}`} onChange={() => toggleTick(live, i.id)} />
            ) : (
              <span />
            )}
            <span className="min-w-0">
              <span className="block truncate font-semibold" title={i.path}>
                {fileName(i.path) || i.path}
              </span>
              {i.block_reason && <span className="block text-[length:var(--fs-xs)] text-faint">Stays: {i.block_reason}</span>}
            </span>
            <span className="tabular-nums font-bold text-paper-dim">{humanBytes(worth(i))}</span>
          </li>
        ))}
      </ul>
      {jobs ? (
        <p className="text-[length:var(--fs-sm)] font-bold" role="status">
          {job?.status === "done" ? `Done. Freed ${humanBytes(job.freed_bytes)}.` : job?.status === "failed" ? (job.error ?? "It didn't work.") : "Working…"}
        </p>
      ) : part.card === "archive" ? (
        <button type="button" className="justify-self-start rounded-full bg-brass px-4 py-1.5 font-extrabold text-on-accent disabled:opacity-40" disabled={bytes === 0} onClick={() => void go()}>
          Move {humanBytes(bytes)}{dest ? ` to ${dest}` : ""}
        </button>
      ) : (
        <HoldPill label={`Delete ${humanBytes(bytes)}`} warning="Deleted for good. There's no Recycle Bin step." disabled={bytes === 0} onConfirm={() => void go()} />
      )}
      {error && <p className="text-[length:var(--fs-sm)] text-err">{error}</p>}
    </>
  );
}

function AreaBody({
  target,
  area,
  onGone,
  onOpen,
  onFindOut,
}: {
  target: Target;
  area: ReviewArea | null;
  onGone: (paths: string[]) => void;
  onOpen: (t: Target) => void;
  onFindOut: () => void;
}) {
  const asking = useStore((s) => s.chatBusy);
  const [why, setWhy] = useState(false);
  const [types, setTypes] = useState<TypeRow[] | null>(null);
  const [dests, setDests] = useState<Destination[]>([]);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    routes.types(target.path).then(setTypes, () => setTypes(null));
    routes.destinations().then(setDests, () => setDests([]));
  }, [target.path]);
  const act = async (kind: ActKind, destination?: string) => {
    setBusy(true);
    try {
      setOutcome({ kind, result: await routes.act(kind, [target.path], destination), error: null });
    } catch (e) {
      setOutcome({ kind, result: null, error: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };
  const keep = async () => {
    setBusy(true);
    try {
      await useReview.getState().answer(target.path, "keep");
      onGone([target.path]);
    } catch (e) {
      setOutcome({ kind: "delete", result: null, error: e instanceof Error ? e.message : String(e) });
      setBusy(false);
    }
  };
  const total = types?.reduce((a, r) => a + r.bytes, 0) ?? 0;
  const mix = types ? typeLine(types, total) : null;
  const say = area ? (area.unknown ? "I don't know what this is yet. I can find out, or you can ask me about it below." : area.why_not) : `A folder inside. ${mix ? `It holds ${mix}.` : ""}`;
  const canShrink = !!area?.actions.includes("compress");
  // Deleting a game library's folder breaks its launcher; games go through the launcher instead.
  const launcherOnly = area?.category === "game-library";
  const started = !!outcome?.result?.job_id;
  return (
    <>
      <Bubble>
        <p>{say}</p>
        {area && (
          <div className="mt-1">
            <button type="button" className="text-[length:var(--fs-xs)] font-bold text-brass hover:underline" aria-expanded={why} onClick={() => setWhy((w) => !w)}>
              {why ? "Hide why" : "Why I think so"}
            </button>
            {why && (
              <ul className="ml-4 mt-1 list-disc text-[length:var(--fs-xs)] text-muted">
                {mix && <li>It holds {mix}.</li>}
                {area.hint && <li>{area.hint}</li>}
                {area.only_copy && <li>It's the only copy Hoardini knows of.</li>}
                {!area.unknown && <li>Hoardini files it under "{area.label}".</li>}
              </ul>
            )}
          </div>
        )}
      </Bubble>
      <Inside path={target.path} size={target.size} onOpen={onOpen} />
      {outcome ? (
        <Result o={outcome} onDone={() => onGone([target.path])} />
      ) : (
        <div className="flex flex-wrap items-start gap-2">
          {(!area || area.unknown) && (
            <button type="button" className="rounded-full px-4 py-1.5 font-extrabold shadow-[inset_0_0_0_1.5px_var(--color-line)] hover:bg-ink-850 disabled:opacity-40" disabled={busy || asking} onClick={onFindOut}>
              Find out what this is
            </button>
          )}
          {launcherOnly ? (
            <p className="basis-full text-[length:var(--fs-sm)] text-muted">Uninstall the games you don't play from Steam or their own launcher. They show up in Your call one by one.</p>
          ) : (
            <HoldPill label={`Delete ${humanBytes(target.size)}`} warning="Deleted for good. There's no Recycle Bin step." disabled={busy} onConfirm={() => void act("delete")} />
          )}
          {canShrink && (
            <button type="button" className="rounded-full px-4 py-1.5 font-extrabold shadow-[inset_0_0_0_1.5px_var(--color-line)] hover:bg-ink-850 disabled:opacity-40" disabled={busy} onClick={() => void act("compress")}>
              Shrink
            </button>
          )}
          {dests.slice(0, 1).map((d) => (
            <button key={d.id} type="button" className="rounded-full px-4 py-1.5 font-extrabold shadow-[inset_0_0_0_1.5px_var(--color-line)] hover:bg-ink-850 disabled:opacity-40" disabled={busy} onClick={() => void act("archive", d.id)}>
              Move to {d.name}
            </button>
          ))}
          {area && (
            <button type="button" className="rounded-full px-4 py-1.5 font-extrabold shadow-[inset_0_0_0_1.5px_var(--color-line)] hover:bg-ink-850 disabled:opacity-40" disabled={busy} onClick={() => void keep()}>
              Keep
            </button>
          )}
        </div>
      )}
      {outcome && !started && !outcome.error && (
        <button type="button" className="justify-self-start text-[length:var(--fs-sm)] font-bold text-brass hover:underline" onClick={() => setOutcome(null)}>
          {(outcome.result?.blocked ?? []).length > 0 ? "Try again" : "Back"}
        </button>
      )}
    </>
  );
}

function AppBody({ row, onGone }: { row: Extract<CallRow, { kind: "app" }>; onGone: (paths: string[]) => void }) {
  const [state, setState] = useState<"idle" | "busy" | "started" | string>("idle");
  const a = row.app;
  const when = a.last_run_ms ? shortDate(a.last_run_ms) : null;
  const say =
    a.source === "steam"
      ? `A Steam game ${when ? `you last played on ${when}` : "nobody has played since Hoardini started watching"}. Steam can install it again any time.`
      : `An app ${when ? `you last opened on ${when}` : "nobody has opened since Hoardini started watching"}${a.publisher ? `, from ${a.publisher}` : ""}.`;
  const go = async () => {
    setState("busy");
    try {
      await routes.uninstallApp(a.id);
      setState("started");
    } catch (e) {
      setState(e instanceof Error ? e.message : String(e));
    }
  };
  return (
    <>
      <Bubble>
        <p>{say}</p>
      </Bubble>
      {state === "started" ? (
        <div className="grid gap-1 text-[length:var(--fs-sm)]" role="status">
          <p className="font-bold">{a.source === "steam" ? "Steam is uninstalling it." : "Its uninstaller is open. Follow its steps."}</p>
          <button type="button" className="justify-self-start font-bold text-brass hover:underline" onClick={() => onGone([a.path])}>
            Done
          </button>
        </div>
      ) : (
        <div className="flex flex-wrap gap-2">
          <button type="button" className="rounded-full bg-brass px-4 py-1.5 font-extrabold text-on-accent disabled:opacity-40" disabled={state === "busy"} onClick={() => void go()}>
            {a.source === "steam" ? "Uninstall in Steam" : "Uninstall"}
          </button>
        </div>
      )}
      {state !== "idle" && state !== "busy" && state !== "started" && <p className="text-[length:var(--fs-sm)] text-err">{state}</p>}
    </>
  );
}

export function DetailSheet({ row, wide, onClose, onGone }: { row: CallRow; wide: boolean; onClose: () => void; onGone: (paths: string[]) => void }) {
  const root: Target | null = row.kind === "part" ? null : { path: row.path, name: row.name, size: row.bytes };
  const [stack, setStack] = useState<Target[]>([]);
  const askRef = useRef<AskHandle>(null);
  const target = stack[stack.length - 1] ?? root;
  const sub = stack.length > 0;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const name = sub ? target!.name : row.name;
  const size = sub ? target!.size : row.bytes;
  const body =
    row.kind === "part" ? (
      <PartBody part={row.part} onDone={() => onGone([])} />
    ) : row.kind === "app" ? (
      <AppBody row={row} onGone={onGone} />
    ) : (
      <AreaBody
        key={target!.path}
        target={target!}
        area={sub || row.kind === "peek" ? null : row.area}
        onGone={onGone}
        onOpen={(t) => setStack((s) => [...s, t])}
        onFindOut={() => askRef.current?.ask(FIND_OUT)}
      />
    );
  const askAbout = row.kind === "part" ? null : (target?.path ?? null);

  const sheet = (
    <div className="grid grid-cols-[minmax(0,1fr)] content-start gap-4 p-5">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          {sub && (
            <button type="button" className="mb-1 text-[length:var(--fs-sm)] font-bold text-brass hover:underline" onClick={() => setStack((s) => s.slice(0, -1))}>
              ← Back
            </button>
          )}
          <h3 className="text-[19px] font-extrabold leading-tight [overflow-wrap:anywhere]">{name}</h3>
          <p className="font-bold tabular-nums text-muted">{humanBytes(size)}</p>
          {target && (
            <div className="mt-0.5 grid gap-2">
              <p className="min-w-0 truncate text-[length:var(--fs-xs)] text-faint" title={target.path}>
                {target.path}
              </p>
              <div>
                <RevealButton path={target.path} label />
              </div>
            </div>
          )}
        </div>
        <button type="button" className="rounded-md p-1 text-muted hover:bg-ink-850 hover:text-paper" aria-label="Close" onClick={onClose}>
          <svg viewBox="0 0 24 24" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
      </div>
      {body}
      {askAbout && <Ask ref={askRef} path={askAbout} name={name} />}
    </div>
  );

  return wide ? (
    <aside aria-label={name} className="w-[420px] shrink-0 overflow-y-auto border-l border-line bg-ink-900 shadow-[-18px_0_40px_-28px_rgba(36,24,44,0.5)]">
      {sheet}
    </aside>
  ) : (
    <div className="fixed inset-0 z-40 flex flex-col bg-black/30" role="dialog" aria-modal="true" aria-label={name} onClick={onClose}>
      <div className="mt-10 min-h-0 flex-1 overflow-y-auto rounded-t-2xl bg-ink-900" onClick={(e) => e.stopPropagation()}>
        <div className="mx-auto mt-2 h-1 w-10 rounded-full bg-line" aria-hidden="true" />
        {sheet}
      </div>
    </div>
  );
}
