import { useEffect, useMemo, useRef, useState } from "react";
import { routes } from "../api/client";
import type { Job, Proposal, ProposalItem, Verdict } from "../api/types";
import { settle } from "../agent/decisions";
import { fileName, humanBytes } from "../format";
import { useStore } from "../store";

const VERDICT_LABEL: Record<Verdict, string> = {
  safe: "safe",
  redownload: "re-download",
  review: "review",
  keep: "keep",
};
const VERDICT_STYLE: Record<Verdict, string> = {
  safe: "bg-ok/15 text-ok border-ok/30",
  redownload: "bg-info/15 text-info border-info/30",
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
  const blocked = !!item.block_reason || item.status === "blocked";
  const verdict = item.verdict;
  return (
    <li className="py-1.5">
      <label className={`flex items-start gap-2 ${blocked || locked ? "opacity-80" : "cursor-pointer"}`}>
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
              {fileName(item.path)}
            </span>
            <span className="ml-auto shrink-0 tabular-nums text-muted">{humanBytes(item.alloc)}</span>
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
          {/* `reason` is the model's; the cap comes from policy code, so they are never joined into one sentence. */}
          {item.cap && !blocked && <span className="block text-[length:var(--fs-xs)] text-brass">Capped to "{VERDICT_LABEL[item.cap]}" by a safety rule</span>}
          {item.block_reason && <span className="block text-[length:var(--fs-xs)] font-semibold text-err">Blocked: {item.block_reason}</span>}
          {item.status === "failed" && item.detail && <span className="block text-[length:var(--fs-xs)] text-err">{item.detail}</span>}
          {(item.status === "done" || item.status === "skipped") && (
            <span className="block text-[length:var(--fs-xs)] text-ok">{item.status === "done" ? "done" : "skipped"}</span>
          )}
        </span>
      </label>
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

export function ProposalCard({ proposalId }: { proposalId: string }) {
  const proposal = useStore((s) => s.proposals[proposalId]);
  const job = useStore((s) => (proposal?.job_id ? s.jobs[proposal.job_id] : undefined));
  const loadProposal = useStore((s) => s.loadProposal);
  const putProposal = useStore((s) => s.putProposal);
  const [picked, setPicked] = useState<Set<number> | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!proposal) void loadProposal(proposalId);
  }, [proposal, proposalId, loadProposal]);

  const selected = useMemo(() => {
    if (!proposal) return new Set<number>();
    return picked ?? new Set(proposal.items.filter((i) => i.selected && !i.block_reason && i.status !== "blocked").map((i) => i.id));
  }, [proposal, picked]);

  if (!proposal) {
    return (
      <div className="my-2 rounded-2xl border border-line bg-ink-850 p-3 text-muted pulse-soft">Loading proposal {proposalId.slice(0, 8)}...</div>
    );
  }

  const pending = proposal.status === "pending";
  const running = proposal.status === "approved" || proposal.status === "running";
  const total = proposal.items.filter((i) => selected.has(i.id)).reduce((a, i) => a + i.alloc, 0);
  const toggle = (id: number) => {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setPicked(next);
  };

  const approve = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await routes.approve(proposalId, [...selected]);
      putProposal({ ...proposal, status: "approved", job_id: res.job_id });
      void loadProposal(proposalId);
      settle(proposalId, res);
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
      settle(proposalId, res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const dest = proposal.destination?.name ?? "destination";
  const heading =
    proposal.kind === "delete"
      ? "Delete"
      : proposal.kind === "archive"
        ? `Archive to ${dest}`
        : proposal.kind === "compress"
          ? `Compress${proposal.format ? ` (${proposal.format})` : ""}`
          : "Remove duplicates";
  const outcome = outcomeLine(proposal, job);
  const compressNote = proposal.kind === "compress" ? compressNoteFor(proposal.format) : null;
  const dedupe = proposal.kind === "dedupe" ? proposal.dedupe : null;
  const permanentDelete = proposal.kind === "delete" || dedupe?.mode === "delete";

  return (
    <div className="fade-up my-2 rounded-2xl border border-brass/40 bg-ink-850 p-3 shadow-sm" data-proposal-id={proposalId} data-status={proposal.status}>
      <div className="flex items-baseline gap-2">
        <span className="font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold">
          {heading} · {proposal.kind === "compress" ? "up to" : "free"} {humanBytes(pending ? total : proposal.total_alloc)}
        </span>
        <span className="ml-auto text-[length:var(--fs-xs)] text-faint">{proposal.status}</span>
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
        {proposal.items.map((it) => (
          <ItemRow key={it.id} item={it} checked={selected.has(it.id)} locked={!pending || busy} onToggle={() => toggle(it.id)} />
        ))}
      </ul>

      {pending && (
        <>
          <div className="mt-2 text-[length:var(--fs-sm)] text-muted tabular-nums">
            {selected.size} of {proposal.items.length} selected · {humanBytes(total)}
          </div>
          {permanentDelete && (
            <p className="mt-1 text-[length:var(--fs-sm)] font-semibold text-err">Deleting is permanent and cannot be undone. Nothing goes to the Recycle Bin.</p>
          )}
          {proposal.kind === "archive" && (
            <p className="mt-1 text-[length:var(--fs-sm)] text-muted">Copies to {dest}, verifies the copy, then deletes the originals.</p>
          )}
          <div className="mt-2 flex flex-wrap items-center gap-2">
            {permanentDelete ? (
              <HoldButton label="Hold to delete" disabled={busy || selected.size === 0} onConfirm={() => void approve()} />
            ) : (
              <button
                type="button"
                className="rounded-lg bg-brass px-3 py-1.5 text-[length:var(--fs-base)] font-bold text-ink-950 hover:opacity-90 disabled:opacity-40"
                disabled={busy || selected.size === 0}
                onClick={() => void approve()}
              >
                {proposal.kind === "archive" ? `Archive to ${dest}, then delete` : proposal.kind === "compress" ? "Compress" : "Remove duplicates"}
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
  );
}
