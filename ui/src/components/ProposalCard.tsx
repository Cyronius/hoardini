import { useEffect, useMemo, useRef, useState } from "react";
import { routes } from "../api/client";
import type { Job, Proposal, ProposalItem, SystemOpView, Verdict } from "../api/types";
import { settle } from "../agent/decisions";
import { fileName, humanBytes } from "../format";
import { heldTicks, holdTicks, useStore } from "../store";
import { ItemTree, RevealButton, Toggle } from "./ItemTree";
import { LearnedChip } from "./LearnedChip";

const VERDICT_LABEL: Record<Verdict, string> = {
  safe: "safe",
  redownload: "re-download",
  synced: "synced",
  review: "review",
  keep: "keep",
};
const VERDICT_STYLE: Record<Verdict, string> = {
  safe: "bg-ok/15 text-ok border-ok/30",
  redownload: "bg-info/15 text-info border-info/30",
  synced: "bg-info/10 text-info border-info/40",
  review: "bg-brass/15 text-brass border-brass/30",
  keep: "bg-ink-800 text-muted border-line",
};

const HOLD_MS = 1200;

export function HoldButton({ label, onConfirm, disabled }: { label: string; onConfirm: () => void; disabled?: boolean }) {
  const [progress, setProgress] = useState(0);
  const raf = useRef<number | null>(null);
  const startedAt = useRef(0);
  const fired = useRef(false);

  const stop = () => {
    if (raf.current != null) cancelAnimationFrame(raf.current);
    raf.current = null;
    setProgress(0);
  };
  const tick = () => {
    const p = Math.min(1, (performance.now() - startedAt.current) / HOLD_MS);
    setProgress(p);
    if (p >= 1) {
      raf.current = null;
      if (!fired.current) {
        fired.current = true;
        onConfirm();
      }
      return;
    }
    raf.current = requestAnimationFrame(tick);
  };
  const start = () => {
    if (disabled || raf.current != null) return;
    fired.current = false;
    startedAt.current = performance.now();
    raf.current = requestAnimationFrame(tick);
  };
  useEffect(() => () => void (raf.current != null && cancelAnimationFrame(raf.current)), []);

  return (
    <button
      type="button"
      disabled={disabled}
      data-hold-progress={progress.toFixed(2)}
      className="relative select-none overflow-hidden rounded-lg border border-err/50 bg-err/10 px-3 py-1.5 text-[length:var(--fs-base)] font-bold text-err disabled:opacity-40"
      onPointerDown={start}
      onPointerUp={stop}
      onPointerLeave={stop}
      onPointerCancel={stop}
      onKeyDown={(e) => {
        if ((e.key === " " || e.key === "Enter") && !e.repeat) {
          e.preventDefault();
          start();
        }
      }}
      onKeyUp={stop}
      onBlur={stop}
    >
      <span className="absolute inset-y-0 left-0 bg-err/30" style={{ width: `${progress * 100}%` }} />
      <span className="relative">{label}</span>
    </button>
  );
}

function compressNoteFor(format: string | null): string | null {
  switch (format) {
    case "zip":
    case "7z":
    case "tar.zst":
      return `Writes a .${format} next to the original, tests it, then deletes the original. Extract it to undo. Savings are an estimate.`;
    case "lzx":
      return "Compresses in place (Windows LZX). Best for data that is not written to again; compact /u undoes it. Savings are an estimate.";
    case "lznt1":
      return "Compresses in place (NTFS compression); compact /u undoes it. Savings are an estimate.";
    default:
      return null;
  }
}

// A Re-downloadable or Online-only reason already quotes the copy note (REVIEW-BUILD).
const saysCopy = (item: ProposalItem) => !!item.reason && item.reason.toLowerCase().includes(item.copy_note!.toLowerCase());

function ItemRow({
  item,
  checked,
  locked,
  onToggle,
}: {
  item: ProposalItem;
  checked: boolean;
  locked: boolean;
  onToggle: () => void;
}) {
  const [open, setOpen] = useState(false);
  const blocked = !!item.block_reason || item.status === "blocked";
  const verdict = item.verdict;
  const name = fileName(item.path);
  return (
    <li className="py-1.5">
      <div className="flex items-start gap-1.5">
        <span className="mt-1 w-4 shrink-0">{item.kind === "dir" && <Toggle open={open} onClick={() => setOpen((o) => !o)} label={name} />}</span>
        <label className={`flex min-w-0 flex-1 items-start gap-2 ${blocked || locked ? "opacity-80" : "cursor-pointer"}`}>
        <input
          type="checkbox"
          className="mt-1 h-4 w-4 shrink-0 accent-[var(--color-brass)]"
          checked={checked}
          disabled={blocked || locked}
          onChange={onToggle}
        />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-2">
            <span className="min-w-0 truncate font-semibold" title={item.path}>
              {name}
            </span>
            <span className="ml-auto shrink-0 tabular-nums text-muted">{humanBytes(item.alloc)}</span>
            {item.small && <span className="shrink-0 rounded-full border border-line px-2 py-px text-[length:var(--fs-xs)] text-faint">small</span>}
            {verdict && (
              <span className={`shrink-0 rounded-full border px-2 py-px text-[length:var(--fs-xs)] font-bold ${VERDICT_STYLE[verdict]}`}>
                {VERDICT_LABEL[verdict]}
              </span>
            )}
          </span>
          <span className="block truncate text-[length:var(--fs-xs)] text-faint" title={item.path}>
            {item.path}
          </span>
          {item.reason && <span className="block text-[length:var(--fs-xs)] text-muted">{item.reason}</span>}
          {item.memory_note && <span className="block text-[length:var(--fs-xs)] text-faint">Noted: {item.memory_note}</span>}
          {item.copy_note && !saysCopy(item) && <span className="block text-[length:var(--fs-xs)] text-faint">Copy: {item.copy_note}</span>}
          {/* A copy check's `redownload` or `synced` cap is already explained by the Copy line. */}
          {item.cap && !blocked && (item.cap === "review" || item.cap === "keep" || !item.copy_note) && (
            <span className="block text-[length:var(--fs-xs)] text-brass">Capped to "{VERDICT_LABEL[item.cap]}" by a safety rule</span>
          )}
          {item.block_reason && <span className="block text-[length:var(--fs-xs)] font-semibold text-err">Blocked: {item.block_reason}</span>}
          {item.status === "failed" && item.detail && <span className="block text-[length:var(--fs-xs)] text-err">{item.detail}</span>}
          {(item.status === "done" || item.status === "skipped") && (
            <span className="block text-[length:var(--fs-xs)] text-ok">{item.status === "done" ? "done" : "skipped"}</span>
          )}
        </span>
        </label>
        {item.status !== "done" && (
          <span className="-mr-1 flex shrink-0 items-center">
            <RevealButton path={item.path} />
          </span>
        )}
      </div>
      {open && (
        <div className="ml-5">
          <ItemTree path={item.path} rootSize={item.alloc} />
        </div>
      )}
    </li>
  );
}

function OpParts({ parts }: { parts: NonNullable<SystemOpView["parts"]> }) {
  const [open, setOpen] = useState(false);
  const n = parts.length;
  return (
    <div className="ml-6 mt-0.5">
      <button type="button" className="inline-flex items-center gap-1 rounded-md py-0.5 pr-1.5 text-[length:var(--fs-xs)] font-bold text-brass hover:bg-brass/10" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span className={`inline-block transition-transform duration-150 ${open ? "rotate-90" : ""}`} aria-hidden="true">
          ▸
        </span>
        {open ? "Hide" : "Show"} the {n} {n === 1 ? "place" : "places"} it clears
      </button>
      {open && (
        <ul className="mb-1 ml-1.5 border-l border-line pl-2.5" aria-label="Places it clears">
          {parts.map((p) => (
            <li key={p.path} className="group flex items-center gap-2 py-0.5 text-[length:var(--fs-xs)]" data-part={p.path}>
              <span className="min-w-0 flex-1 truncate text-paper-dim [direction:rtl] text-left" title={p.path}>
                <bdi>{p.path}</bdi>
              </span>
              <span className="shrink-0 tabular-nums text-muted">{humanBytes(p.size)}</span>
              <RevealButton path={p.path} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function OpRow({ item, checked, locked, onToggle }: { item: ProposalItem; checked: boolean; locked: boolean; onToggle: () => void }) {
  const op = item.op!;
  return (
    <li className="py-1.5" data-op={op.op}>
      <label className={`flex items-start gap-2 ${locked ? "opacity-80" : "cursor-pointer"}`}>
        <input type="checkbox" className="mt-1 h-4 w-4 shrink-0 accent-[var(--color-brass)]" checked={checked} disabled={locked} onChange={onToggle} />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-2">
            <span className="min-w-0 font-semibold" title={item.path}>
              {op.label}
            </span>
            <span className="ml-auto shrink-0 tabular-nums text-muted">{op.estimate_known ? humanBytes(item.alloc) : "size unknown"}</span>
            {op.irreversible && (
              <span className="shrink-0 rounded-full border border-err/40 px-2 py-px text-[length:var(--fs-xs)] font-bold text-err">cannot be undone</span>
            )}
          </span>
          <span className="block text-[length:var(--fs-xs)] text-muted">{op.explain}</span>
          {item.reason && <span className="block text-[length:var(--fs-xs)] text-faint">{item.reason}</span>}
          {item.status === "failed" && item.detail && <span className="block text-[length:var(--fs-xs)] text-err">{item.detail}</span>}
          {(item.status === "done" || item.status === "skipped") && (
            <span className="block text-[length:var(--fs-xs)] text-ok">
              {item.status === "done" ? "done" : "skipped"}
              {item.detail ? `: ${item.detail}` : ""}
            </span>
          )}
        </span>
      </label>
      {op.parts && op.parts.length > 0 && item.status !== "done" && <OpParts parts={op.parts} />}
    </li>
  );
}

function JobProgress({ job }: { job: Job }) {
  const p = job.progress;
  const frac = p.total_bytes > 0 ? Math.min(1, p.done_bytes / p.total_bytes) : 0;
  return (
    <div className="mt-2">
      <div className="h-2 overflow-hidden rounded-full bg-ink-800">
        <div className="h-full rounded-full bg-brass transition-all" style={{ width: `${frac * 100}%` }} />
      </div>
      <div className="mt-1 flex gap-2 text-[length:var(--fs-xs)] text-muted">
        <span className="tabular-nums">
          {humanBytes(p.done_bytes)} of {humanBytes(p.total_bytes)} · {p.done_items}/{p.total_items} items
        </span>
        {p.current && <span className="min-w-0 truncate">{p.current}</span>}
      </div>
    </div>
  );
}

function outcomeLine(p: Proposal, job: Job | undefined): { text: string; tone: string } | null {
  switch (p.status) {
    case "declined":
      return { text: "Declined. Nothing was changed.", tone: "text-muted" };
    case "superseded":
      return { text: "Replaced by a newer review. Nothing was changed.", tone: "text-muted" };
    case "done":
      return { text: `Done${job ? `. Freed ${humanBytes(job.freed_bytes)}` : ""}.`, tone: "text-ok" };
    case "partial":
      return { text: `Partly done${job ? `. Freed ${humanBytes(job.freed_bytes)}` : ""}. Some items were skipped or failed.`, tone: "text-brass" };
    case "failed":
      return { text: job?.error ? `Failed: ${job.error}` : "Failed.", tone: "text-err" };
    default:
      return null;
  }
}

const VERB: Record<Proposal["kind"], string> = {
  delete: "Deleted",
  archive: "Archived",
  compress: "Compressed",
  dedupe: "Deduplicated",
  system_cleanup: "Cleaned up",
  online_only: "Made online-only",
};
const ENDED: Proposal["status"][] = ["declined", "done", "partial", "failed", "superseded"];

function collapsedLine(p: Proposal, job: Job | undefined): string {
  const n = (k: number) => `${k} item${k === 1 ? "" : "s"}`;
  if (p.status === "declined") return `Declined · ${n(p.items.length)}`;
  if (p.status === "superseded") return `Replaced by a newer review · ${n(p.items.length)}`;
  const done = p.items.filter((i) => i.status === "done");
  const freed = job ? job.freed_bytes : done.reduce((a, i) => a + i.alloc, 0);
  if (p.status === "failed" && done.length === 0) return `${VERB[p.kind]} nothing · failed${job?.error ? `: ${job.error}` : ""}`;
  const count = p.status === "partial" ? `${done.length} of ${p.items.length} items` : n(done.length);
  return `${VERB[p.kind]} ${count} · ${p.kind === "compress" ? "saved" : "freed"} ${humanBytes(freed)}`;
}

// `title` names a review card (REVIEW-BUILD); chat cards are headed by their kind.
export function ProposalCard({ proposalId, title, note }: { proposalId: string; title?: string; note?: string }) {
  const proposal = useStore((s) => s.proposals[proposalId]);
  return (
    <>
      <ProposalBody proposalId={proposalId} title={title} note={note} />
      {proposal?.follow_up_id && <ProposalCard key={proposal.follow_up_id} proposalId={proposal.follow_up_id} title={title} />}
    </>
  );
}

function ProposalBody({ proposalId, title, note }: { proposalId: string; title?: string; note?: string }) {
  const proposal = useStore((s) => s.proposals[proposalId]);
  const job = useStore((s) => (proposal?.job_id ? s.jobs[proposal.job_id] : undefined));
  const loadProposal = useStore((s) => s.loadProposal);
  const putProposal = useStore((s) => s.putProposal);
  const [picked, setPicked] = useState<Set<number> | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const sending = useRef(false);

  useEffect(() => {
    if (!proposal) void loadProposal(proposalId);
  }, [proposal, proposalId, loadProposal]);

  // A review card keeps its ticks on the server, so Ready to free and a reload agree with them (SRV-PROPOSAL-SELECTION).
  const persisted = !!proposal?.review_id;
  const selected = useMemo(() => {
    if (!proposal) return new Set<number>();
    return (persisted ? null : picked) ?? new Set(proposal.items.filter((i) => i.selected && !i.block_reason && i.status !== "blocked").map((i) => i.id));
  }, [proposal, picked, persisted]);

  if (!proposal) {
    return (
      <div className="my-2 rounded-2xl border border-line bg-ink-850 p-3 text-muted pulse-soft">Loading proposal {proposalId.slice(0, 8)}...</div>
    );
  }

  const pending = proposal.status === "pending";
  const running = proposal.status === "approved" || proposal.status === "running";
  // A compress item frees what its estimate saves, not its size (REVIEW-BUILD).
  const worth = (i: ProposalItem) => i.saves ?? i.alloc;
  const total = proposal.items.filter((i) => selected.has(i.id)).reduce((a, i) => a + worth(i), 0);
  const toggle = (id: number) => {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    if (!persisted) return setPicked(next);
    holdTicks(proposalId, next);
    putProposal({ ...proposal, touched: true });
    setError(null);
    if (!sending.current) void flushTicks();
  };
  // One PUT in flight per card; ticks made meanwhile go in the next one, so replies never land out of order.
  const flushTicks = async () => {
    sending.current = true;
    try {
      for (let want = heldTicks(proposalId); want; want = heldTicks(proposalId)) {
        const p = await routes.selectItems(proposalId, [...want]);
        if (heldTicks(proposalId) !== want) continue;
        holdTicks(proposalId, null);
        putProposal(p);
      }
    } catch (e) {
      holdTicks(proposalId, null);
      setError(e instanceof Error ? e.message : String(e));
      void loadProposal(proposalId);
    } finally {
      sending.current = false;
    }
  };

  const approve = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await routes.approve(proposalId, [...selected]);
      putProposal({ ...proposal, status: "approved", job_id: res.job_id });
      void loadProposal(proposalId);
      if (!proposal.parent_id && !proposal.review_id) settle(proposalId, res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const decline = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await routes.decline(proposalId);
      putProposal({ ...proposal, status: "declined" });
      if (!proposal.parent_id && !proposal.review_id) settle(proposalId, res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const dest = proposal.destination?.name ?? "destination";
  const system = proposal.kind === "system_cleanup";
  const heading = title ?? (
    proposal.kind === "delete"
      ? "Delete"
      : proposal.kind === "archive"
        ? `Archive to ${dest}`
        : proposal.kind === "compress"
          ? `Compress${proposal.format ? ` (${proposal.format})` : ""}`
          : system
            ? "Clean up with Windows"
            : proposal.kind === "online_only"
              ? "Make online-only"
              : "Remove duplicates");
  const outcome = outcomeLine(proposal, job);
  const compressNote = proposal.kind === "compress" ? compressNoteFor(proposal.format) : null;
  const dedupe = proposal.kind === "dedupe" ? proposal.dedupe : null;
  const ended = ENDED.includes(proposal.status) || (!!job && ["done", "failed", "cancelled"].includes(job.status));
  const pickedOps = proposal.items.filter((i) => selected.has(i.id) && i.op).map((i) => i.op!);
  const unknownSize = pickedOps.some((o) => !o.estimate_known);
  const irreversible = pickedOps.filter((o) => o.irreversible);
  const permanentDelete = proposal.kind === "delete" || dedupe?.mode === "delete" || irreversible.length > 0;
  // "free 0 B" reads like a broken card; with nothing ticked the heading says what could go.
  const nothingPicked = pending && selected.size === 0;
  const choosable = proposal.items.filter((i) => !i.block_reason && i.status === "pending").reduce((a, i) => a + worth(i), 0);
  const saves = proposal.kind === "compress" && proposal.items.some((i) => i.saves != null);

  const marks = { "data-proposal-id": proposalId, "data-status": proposal.status, "data-follow-up": proposal.follow_up_id ?? undefined };
  const learned = ended
    ? (proposal.learned ?? []).map((l) => (
        <LearnedChip
          key={l.memory_id}
          text={l.text}
          onUndo={() => routes.undoLearned(proposalId, l.memory_id)}
          onPin={(pinned) => routes.updateMemory(l.memory_id, { pinned })}
          confirmed={l.confirmed}
          pinned={l.pinned}
        />
      ))
    : null;
  if (ended && !open) {
    return (
      <>
      <button
        type="button"
        {...marks}
        data-collapsed=""
        className="my-2 flex w-full items-baseline gap-2 rounded-xl border border-line bg-ink-850 px-3 py-1.5 text-left text-[length:var(--fs-sm)] text-muted hover:text-paper"
        onClick={() => setOpen(true)}
      >
        <span className="min-w-0 flex-1 truncate">{collapsedLine(proposal, job)}</span>
        <span className="shrink-0 text-faint">details ▸</span>
      </button>
      {learned}
      </>
    );
  }

  return (
    <>
    <div className="@container fade-up my-2 rounded-2xl border border-brass/40 bg-ink-850 p-3 shadow-sm" {...marks}>
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <span className="font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold">
          {heading} · {saves ? "saves about" : proposal.kind === "compress" || nothingPicked ? "up to" : system && unknownSize ? "free at least" : "free"}{" "}
          {humanBytes(nothingPicked ? choosable : pending ? total : saves ? proposal.items.reduce((a, i) => a + worth(i), 0) : proposal.total_alloc)}
        </span>
        {note && (
          <span data-note="" className="shrink-0 rounded-full border border-line bg-ink-900 px-2 py-px text-[length:var(--fs-xs)] font-semibold text-muted">
            {note}
          </span>
        )}
        {ended ? (
          <button type="button" className="ml-auto text-[length:var(--fs-xs)] text-faint hover:text-paper" onClick={() => setOpen(false)}>
            hide ▴
          </button>
        ) : (
          <span className="ml-auto text-[length:var(--fs-xs)] text-faint">{proposal.status}</span>
        )}
      </div>
      {proposal.summary && <p className="mt-1 text-[length:var(--fs-sm)] text-muted">{proposal.summary}</p>}
      {dedupe && (
        <p className="mt-1 text-[length:var(--fs-sm)] text-muted" title={dedupe.keep}>
          Keeps <span className="font-semibold text-paper">{fileName(dedupe.keep)}</span>. The listed copies are{" "}
          {dedupe.mode === "hardlink" ? "replaced with hard links to it (every path keeps working)" : "deleted"} after their contents are re-checked.
        </p>
      )}
      {compressNote && <p className="mt-1 text-[length:var(--fs-sm)] text-muted">{compressNote}</p>}

      <ul className="mt-2 divide-y divide-line-soft rounded-xl border border-line-soft bg-ink-900 px-2.5">
        {proposal.items.map((it) =>
          it.op ? (
            <OpRow key={it.id} item={it} checked={selected.has(it.id)} locked={!pending || busy} onToggle={() => toggle(it.id)} />
          ) : (
            <ItemRow key={it.id} item={it} checked={selected.has(it.id)} locked={!pending || busy} onToggle={() => toggle(it.id)} />
          ),
        )}
      </ul>

      <div data-proposal-foot="">
      {pending && (
        <>
          <div className="mt-2 text-[length:var(--fs-sm)] text-muted tabular-nums">
            {selected.size} of {proposal.items.length} selected · {humanBytes(total)}
          </div>
          {permanentDelete && !system && (
            <p className="mt-1 text-[length:var(--fs-sm)] font-semibold text-err">Deleting is permanent and cannot be undone. Nothing goes to the Recycle Bin.</p>
          )}
          {irreversible.some((o) => o.op === "restore_points") && (
            <p className="mt-1 text-[length:var(--fs-sm)] font-semibold text-err">Deleted restore points cannot be brought back.</p>
          )}
          {irreversible.some((o) => o.op === "recycle_bin") && (
            <p className="mt-1 text-[length:var(--fs-sm)] font-semibold text-err">Emptying the Recycle Bin is permanent; its files cannot be restored.</p>
          )}
          {pickedOps.some((o) => o.elevated) && (
            <p className="mt-1 text-[length:var(--fs-sm)] text-muted">Windows asks for administrator permission once, then runs its own tools. The space freed is measured as free space before and after.</p>
          )}
          {proposal.kind === "archive" && (
            <p className="mt-1 text-[length:var(--fs-sm)] text-muted">Copies to {dest}, verifies the copy, then deletes the originals.</p>
          )}
          {proposal.kind === "online_only" && (
            <p className="mt-1 text-[length:var(--fs-sm)] text-muted">
              Frees the copy on this PC. Each file stays in its cloud folder and downloads again when you open it. Files not in sync yet are left as they are.
            </p>
          )}
          <div className="mt-2 flex flex-wrap items-center gap-2">
            {permanentDelete ? (
              <HoldButton label={system ? "Hold to clean up" : "Hold to delete"} disabled={busy || selected.size === 0} onConfirm={() => void approve()} />
            ) : (
              <button
                type="button"
                className="rounded-lg bg-brass px-3 py-1.5 text-[length:var(--fs-base)] font-bold text-on-accent hover:opacity-90 disabled:opacity-40"
                disabled={busy || selected.size === 0}
                onClick={() => void approve()}
              >
                {proposal.kind === "archive"
                  ? `Archive to ${dest}, then delete`
                  : proposal.kind === "compress"
                    ? "Compress"
                    : system
                      ? "Clean up"
                      : proposal.kind === "online_only"
                        ? "Make online-only"
                        : "Remove duplicates"}
              </button>
            )}
            <button
              type="button"
              className="rounded-lg border border-line px-3 py-1.5 text-[length:var(--fs-base)] font-semibold text-muted hover:text-paper disabled:opacity-40"
              disabled={busy}
              onClick={() => void decline()}
            >
              Cancel
            </button>
          </div>
        </>
      )}

      {running && (job ? <JobProgress job={job} /> : <div className="mt-2 text-[length:var(--fs-sm)] text-muted pulse-soft">Working...</div>)}
      {outcome && <div className={`mt-2 text-[length:var(--fs-sm)] font-semibold ${outcome.tone}`}>{outcome.text}</div>}
      {error && <div className="mt-2 text-[length:var(--fs-sm)] text-err">{error}</div>}
      </div>
    </div>
    {learned}
    </>
  );
}
