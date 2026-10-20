import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { routes } from "../api/client";
import type { Job, Proposal, ProposalItem, ReviewArea } from "../api/types";
import { fileName, humanBytes } from "../format";
import { useReview } from "../review/state";
import { useStore } from "../store";
import { selectedVolume } from "../components/mascot/girth";
import { DetailSheet } from "./DetailSheet";
import { choosable, homeOf, worth, type CallRow, type Group, type GroupId, type Part } from "./groups";
import { HoldPill } from "./HoldPill";
import { approveParts, toggleTick } from "./ticks";

const ENDED: Job["status"][] = ["done", "failed", "cancelled"];
const SHOWN = 6;

type Run = { jobs: string[]; errors: string[] };

function useHomeSync() {
  const root = useStore((s) => selectedVolume(s.status?.volumes)?.root);
  const statusId = useStore((s) => s.status?.review?.id);
  const statusBuilding = useStore((s) => s.status?.review_building);
  const lastRefresh = useStore((s) => s.status?.index.last_refresh_ms);
  const connected = useStore((s) => s.connected);
  const { open, refetch } = useReview.getState();
  const noIndex = useReview((s) => s.status === "no-index");
  const shownId = useReview((s) => s.main?.review.id);
  const building = useReview((s) => s.building);

  useEffect(() => {
    if (root && connected) void open();
  }, [root, connected, open]);
  useEffect(() => {
    if (noIndex && lastRefresh) void open();
  }, [noIndex, lastRefresh, open]);
  useEffect(() => {
    if (statusId && statusId !== shownId && !noIndex) void refetch();
  }, [statusId, shownId, noIndex, refetch]);
  // Only the end of a build the stream saw counts: its first frame may predate the build `open` started.
  const wasBuilding = useRef(false);
  useEffect(() => {
    const ended = wasBuilding.current && statusBuilding === false;
    wasBuilding.current = !!statusBuilding;
    if (ended && building) void refetch();
  }, [building, statusBuilding, refetch]);
}

const drive = (root: string) => root.replace(/\\$/, "");
const things = (n: number) => `${n} big ${n === 1 ? "thing is" : "things are"}`;

function itemLabel(i: ProposalItem): string {
  return i.op?.label ?? (fileName(i.path) || i.path);
}

function PartItems({ part, live, onPeek }: { part: Part; live: Proposal; onPeek: (i: ProposalItem) => void }) {
  const [all, setAll] = useState(false);
  const rows = live.items.filter((i) => i.status === "pending");
  const shown = all ? rows : rows.slice(0, SHOWN);
  return (
    <div className="grid gap-0.5">
      <div className="mt-2 flex items-baseline gap-2">
        <span className="font-bold">{part.name}</span>
        <span className="text-[length:var(--fs-xs)] text-faint">{part.note}</span>
      </div>
      <ul className="grid">
        {shown.map((i) => (
          <li key={i.id} className="grid grid-cols-[20px_minmax(0,1fr)] items-start gap-x-2.5 border-t border-line-soft py-1.5">
            {choosable(i) ? (
              <input
                type="checkbox"
                className="mt-1 h-4 w-4 accent-[var(--color-brass)]"
                checked={i.selected}
                aria-label={`Include ${itemLabel(i)}`}
                onChange={() => toggleTick(live, i.id)}
              />
            ) : (
              <span />
            )}
            <button type="button" className="group grid w-full grid-cols-[minmax(0,1fr)_auto] items-start gap-x-2.5 rounded-md text-left hover:bg-ink-850" aria-label={`Look inside ${itemLabel(i)}`} title={`Look inside ${i.path}`} onClick={() => onPeek(i)}>
              <span className="min-w-0">
                <span className="block truncate font-semibold" title={i.path}>
                  {itemLabel(i)}
                </span>
                <span className="block truncate text-[length:var(--fs-xs)] text-faint" title={i.path}>
                  {i.path}
                </span>
                {i.block_reason ? (
                  <span className="block text-[length:var(--fs-xs)] text-faint">Stays: {i.block_reason}</span>
                ) : (
                  i.reason && <span className="block truncate text-[length:var(--fs-xs)] text-muted">{i.reason}</span>
                )}
              </span>
              <span className="flex items-center gap-2 text-[length:var(--fs-sm)] font-bold text-paper-dim group-hover:text-brass">
                <span className="tabular-nums">{i.op && !i.op.estimate_known ? "size unknown" : humanBytes(worth(i))}</span>
                <svg viewBox="0 0 24 24" className="h-4 w-4 text-faint group-hover:text-brass" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
                  <path d="M9 6l6 6-6 6" />
                </svg>
              </span>
            </button>
          </li>
        ))}
      </ul>
      {rows.length > SHOWN && (
        <button type="button" className="justify-self-start text-[length:var(--fs-sm)] font-bold text-brass hover:underline" onClick={() => setAll((a) => !a)}>
          {all ? "Show fewer" : `Show ${rows.length - SHOWN} more`}
        </button>
      )}
    </div>
  );
}

// Nested markers let the mascot's trip find this block for any of its parts' jobs (UI-MASCOT-JOBS).
function Markers({ ids, children }: { ids: string[]; children: ReactNode }) {
  return ids.reduceRight<ReactNode>((inner, id) => <div data-proposal-id={id}>{inner}</div>, children);
}

function GroupBlock({ id, title, group, run, onRun, onPeek }: { id: GroupId; title: string; group: Group; run: Run | undefined; onRun: (r: Run) => void; onPeek: (i: ProposalItem) => void }) {
  const proposals = useStore((s) => s.proposals);
  const jobs = useStore((s) => s.jobs);
  const mine = (run?.jobs ?? []).map((j) => jobs[j]).filter((j): j is Job => !!j);
  const working = !!run && run.jobs.length > 0 && (mine.length < run.jobs.length || mine.some((j) => !ENDED.includes(j.status)));
  const done = !!run && !working && run.jobs.length > 0;
  const freed = mine.reduce((a, j) => a + j.freed_bytes, 0);
  const total = mine.reduce((a, j) => a + j.progress.total_bytes, 0);
  const moved = mine.reduce((a, j) => a + j.progress.done_bytes, 0);
  const failed = mine.filter((j) => j.status === "failed");
  useEffect(() => {
    if (done) void useReview.getState().changed();
  }, [done]);
  const go = async () => {
    const r = await approveParts(group.parts);
    onRun(r);
  };
  const size = `${group.atLeast ? "at least " : ""}${humanBytes(group.bytes)}`;

  let action: ReactNode;
  if (working) {
    action = (
      <span className="text-[length:var(--fs-sm)] font-bold text-paper-dim tabular-nums" role="status">
        {id === "junk" ? "Clearing" : "Freeing"}… {total > 0 ? `${Math.round((moved / total) * 100)}%` : ""}
      </span>
    );
  } else if (done) {
    action = (
      <span className="text-[length:var(--fs-base)] font-extrabold" role="status">
        Freed {humanBytes(freed)}
      </span>
    );
  } else if (id === "junk") {
    action = <HoldPill label={`Clear ${humanBytes(group.bytes)}`} warning="Deleted for good. There's no Recycle Bin step." disabled={group.bytes === 0 && !group.atLeast} onConfirm={() => void go()} />;
  } else {
    action = (
      <button type="button" className="rounded-full bg-ink-800 px-4 py-1.5 text-[length:var(--fs-base)] font-extrabold text-paper hover:bg-line disabled:opacity-40" disabled={group.bytes === 0} onClick={() => void go()}>
        Free {humanBytes(group.bytes)}
      </button>
    );
  }

  return (
    <Markers ids={group.parts.map((p) => p.proposal.id)}>
      <section aria-label={title} className="grid gap-1.5">
        <div className="flex flex-wrap items-start gap-x-3 gap-y-1">
          <h2 className="pt-1 text-[length:var(--fs-md)] font-extrabold">{title}</h2>
          {!done && !working && <span className="pt-1 font-bold tabular-nums text-muted">{size}</span>}
          <div className="ml-auto" data-proposal-foot="">
            {action}
          </div>
        </div>
        {(run?.errors.length ?? 0) > 0 && <p className="text-[length:var(--fs-sm)] text-err">{run!.errors[0]}</p>}
        {done && failed.length > 0 && <p className="text-[length:var(--fs-sm)] text-err">{failed[0]!.error ?? "Part of it failed."}</p>}
        {group.parts.map((p) => <PartItems key={p.proposal.id} part={p} live={proposals[p.proposal.id] ?? p.proposal} onPeek={onPeek} />)}
      </section>
    </Markers>
  );
}

const CALLS_SHOWN = 8;

function CallRows({ rows, selected, onOpen }: { rows: CallRow[]; selected: string | null; onOpen: (key: string) => void }) {
  const [all, setAll] = useState(false);
  const shown = all ? rows : rows.slice(0, CALLS_SHOWN);
  return (
    <>
    <ul className="grid">
      {shown.map((r) => (
        <li key={r.key}>
          <button
            type="button"
            className={`grid w-full grid-cols-[minmax(0,1fr)_auto_16px] items-center gap-x-3 border-t border-line-soft px-1 py-2.5 text-left hover:bg-ink-850 ${selected === r.key ? "bg-ink-850" : ""}`}
            aria-current={selected === r.key ? "true" : undefined}
            onClick={() => onOpen(r.key)}
          >
            <span className="min-w-0">
              <span className="block truncate text-[length:var(--fs-base)] font-bold">{r.name}</span>
              <span className="block truncate text-[length:var(--fs-sm)] text-muted">{r.desc}</span>
            </span>
            <span className="tabular-nums font-bold text-paper-dim">{humanBytes(r.bytes)}</span>
            <svg viewBox="0 0 24 24" className="h-4 w-4 text-faint" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
              <path d="M9 6l6 6-6 6" />
            </svg>
          </button>
        </li>
      ))}
    </ul>
    {rows.length > CALLS_SHOWN && (
      <button type="button" className="justify-self-start text-[length:var(--fs-sm)] font-bold text-brass hover:underline" onClick={() => setAll((a) => !a)}>
        {all ? "Show fewer" : `Show ${rows.length - CALLS_SHOWN} more`}
      </button>
    )}
    </>
  );
}

function Kept({ kept }: { kept: ReviewArea[] }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const bringBack = async (a: ReviewArea) => {
    setBusy(a.path);
    try {
      await routes.forgetAnswer(a.path);
      await useReview.getState().runNow();
    } finally {
      setBusy(null);
    }
  };
  if (kept.length === 0) return null;
  return (
    <div className="grid gap-1 text-[length:var(--fs-sm)] text-muted">
      <p>
        {kept.length} {kept.length === 1 ? "thing" : "things"} you kept ·{" "}
        <button type="button" className="font-bold text-brass hover:underline" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
          {open ? "Hide" : "Show"}
        </button>
      </p>
      {open && (
        <ul className="grid">
          {kept.map((a) => (
            <li key={a.path} className="flex items-center gap-3 border-t border-line-soft py-1.5">
              <span className="min-w-0 flex-1 truncate" title={a.path}>
                <span className="font-semibold text-paper">{fileName(a.path) || a.path}</span> · {humanBytes(a.size)}
              </span>
              <button type="button" className="font-bold text-brass hover:underline disabled:opacity-40" disabled={busy != null} onClick={() => void bringBack(a)}>
                {busy === a.path ? "Bringing back…" : "Bring back"}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Message({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="fade-up grid gap-1.5 py-6">
      <h1 className="font-[family-name:var(--font-display)] text-[28px] font-bold leading-tight tracking-tight">{title}</h1>
      {children && <div className="text-paper-dim">{children}</div>}
    </div>
  );
}

function useWide(min = 960): boolean {
  const [wide, setWide] = useState(() => window.innerWidth >= min);
  useEffect(() => {
    const on = () => setWide(window.innerWidth >= min);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, [min]);
  return wide;
}

export function Home() {
  useHomeSync();
  const { main, bare, status, error } = useReview();
  const proposals = useStore((s) => s.proposals);
  const vol = useStore((s) => selectedVolume(s.status?.volumes));
  const [gone, setGone] = useState<Set<string>>(() => new Set());
  const [runs, setRuns] = useState<Partial<Record<GroupId, Run>>>({});
  const [sel, setSel] = useState<string | null>(null);
  const [peek, setPeek] = useState<CallRow | null>(null);
  const wide = useWide();
  const reviewId = main?.review.id;
  // A newer review lists what is left, so finished runs and acted paths start over with it.
  useEffect(() => {
    setRuns((r) => (Object.keys(r).length ? {} : r));
  }, [reviewId]);

  const home = useMemo(() => (main ? homeOf(main, proposals, gone) : null), [main, proposals, gone]);
  const clean = !!home && home.junk.parts.length === 0 && home.keep.parts.length === 0 && home.calls.length === 0 && !runs.junk && !runs.keep;
  useEffect(() => {
    useStore.setState({ homeClean: clean });
  }, [clean]);
  const row = peek ?? home?.calls.find((c) => c.key === sel) ?? null;
  useEffect(() => {
    if (sel && home && !row) setSel(null);
  }, [sel, home, row]);

  const look = (i: ProposalItem) => {
    setSel(null);
    setPeek({ kind: "peek", key: `peek:${i.id}`, name: itemLabel(i), desc: "", bytes: i.alloc, path: i.path });
  };
  const openCall = (key: string) => {
    setPeek(null);
    setSel(key);
  };

  let body: ReactNode;
  if (status === "no-index") {
    body = (
      <Message title={`Hoardini is taking his first look at ${vol ? drive(vol.root) : "your drive"}.`}>
        It takes a few minutes the first time. The list fills in when he's done.
      </Message>
    );
  } else if (status === "error" && !main) {
    body = (
      <Message title="Hoardini couldn't look at the drive.">
        <p>{error}</p>
        <button type="button" className="mt-2 font-bold text-brass hover:underline" onClick={() => void useReview.getState().open()}>
          Try again
        </button>
      </Message>
    );
  } else if (!home) {
    body = (
      <Message title={bare ? "Hoardini is looking around…" : "Looking over the drive…"}>
        The list fills in as he finds things. Nothing changes while he looks.
      </Message>
    );
  } else {
    const showJunk = home.junk.parts.length > 0 || !!runs.junk;
    const showKeep = home.keep.parts.length > 0 || !!runs.keep;
    const clean = !showJunk && !showKeep && home.calls.length === 0;
    body = clean ? (
      <Message title={`All clean. ${vol ? `${humanBytes(vol.free_bytes)} free on ${drive(vol.root)}.` : ""}`}>
        Nothing big is waiting. I'll keep an eye on things.
        <div className="mt-4">
          <Kept kept={home.kept} />
        </div>
      </Message>
    ) : (
      <>
        <div className="fade-up grid gap-1">
          <h1 className="font-[family-name:var(--font-display)] text-[30px] font-bold leading-[1.15] tracking-tight">
            {home.freeable > 0 ? `You can free ${humanBytes(home.freeable)} without losing anything.` : home.calls.length > 0 ? `${things(home.calls.length)} waiting for your call.` : "Clearing up."}
          </h1>
          {home.freeable > 0 && home.calls.length > 0 && <p className="text-paper-dim">And {things(home.calls.length)} waiting for your call.</p>}
        </div>
        {showJunk && <GroupBlock id="junk" title="Junk you won't miss" group={home.junk} run={runs.junk} onRun={(r) => setRuns((x) => ({ ...x, junk: r }))} onPeek={look} />}
        {showKeep && <GroupBlock id="keep" title="Free space, keep the files" group={home.keep} run={runs.keep} onRun={(r) => setRuns((x) => ({ ...x, keep: r }))} onPeek={look} />}
        {home.calls.length > 0 && (
          <section aria-label="Your call" className="grid gap-1.5">
            <div className="flex items-baseline gap-3">
              <h2 className="text-[length:var(--fs-md)] font-extrabold">Your call</h2>
              <span className="font-bold text-muted">
                {home.calls.length} big {home.calls.length === 1 ? "thing" : "things"}
              </span>
            </div>
            <CallRows rows={home.calls} selected={sel} onOpen={openCall} />
          </section>
        )}
        <Kept kept={home.kept} />
      </>
    );
  }

  const sheet = row && (
    <DetailSheet
      key={row.key}
      row={row}
      wide={wide}
      onClose={() => {
        setSel(null);
        setPeek(null);
      }}
      onGone={(paths) => {
        setPeek(null);
        void useReview.getState().changed();
        setGone((g) => new Set([...g, ...paths.map((p) => p.toLowerCase())]));
        setSel(null);
      }}
    />
  );

  return (
    <div className="flex min-h-0 flex-1">
      <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden" role="region" aria-label="Home">
        <div className="mx-auto grid w-full max-w-2xl gap-7 px-5 pb-12 pt-7">{body}</div>
      </div>
      {sheet}
    </div>
  );
}
