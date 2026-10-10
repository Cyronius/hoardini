import { useEffect, useMemo, useRef, useState } from "react";
import type { Proposal, ReviewCardRef, ReviewView as Review } from "../api/types";
import { Hello } from "../components/Greeting";
import { letter as drive } from "../components/header/Volumes";
import { RefreshIcon, SpinnerIcon } from "../components/icons";
import { selectedVolume } from "../components/mascot/girth";
import { OnlineChecksConsent } from "../components/OnlineChecksConsent";
import { ProposalCard } from "../components/ProposalCard";
import { humanAge, humanBytes, isUnder, samePath } from "../format";
import { useStore } from "../store";
import { LargeAreas, SectionHead, WhatIsCard } from "./LargeAreas";
import { chainOf, orderCards, readyBytes } from "./order";
import { RarelyUsedApps } from "./RarelyUsedApps";
import { useReview, type Built } from "./state";

// Opens the drive's review at start and on a drive switch; a rebuild elsewhere shows up as a new status.review id.
function useReviewSync() {
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

// Review now has its own "Reviewing…", so its build is not shown twice.
function useBuilding(): boolean {
  const building = useReview((s) => s.building);
  const running = useReview((s) => s.running);
  const statusBuilding = useStore((s) => s.status?.review_building);
  return !running && (building || !!statusBuilding);
}

function useNow(ms: number) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [ms]);
  return now;
}

// Ready to free follows the ticks of every card on screen, follow-ups included (REVIEW-TRIGGER's ready_bytes).
function useReady(cards: ReviewCardRef[]): number {
  return useStore((s) => readyBytes(cards.flatMap((c) => chainOf(c.proposal_id, s.proposals))));
}

function SpaceBar({ used, free, ready }: { used: number; free: number; ready: number | null }) {
  const total = used + free;
  if (total <= 0) return null;
  const pct = (n: number) => `${(n / total) * 100}%`;
  const r = Math.min(ready ?? 0, used);
  const label = ready == null ? `${humanBytes(used)} used, ${humanBytes(free)} free` : `${humanBytes(used)} used, ${humanBytes(r)} of it ready to free, ${humanBytes(free)} free`;
  return (
    <div className="mt-3">
      <div className="flex h-3 overflow-hidden rounded-full bg-ink-800" role="img" aria-label={label}>
        <span className="h-full bg-muted/45" style={{ width: pct(used - r) }} />
        <span className="review-ready h-full" style={{ width: pct(r) }} />
        <span className="h-full bg-ok/30" style={{ width: pct(free) }} />
      </div>
      <div className="mt-1.5 flex flex-wrap gap-x-4 gap-y-0.5 text-[length:var(--fs-xs)] text-muted">
        <Key cls="bg-muted/45" label={`Used ${humanBytes(used)}`} />
        {ready != null && <Key cls="review-ready" label={`Ready to free ${humanBytes(r)}`} />}
        <Key cls="bg-ok/30" label={`Free ${humanBytes(free)} of ${humanBytes(total)}`} />
      </div>
    </div>
  );
}

function Key({ cls, label }: { cls: string; label: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 tabular-nums">
      <span className={`inline-block h-2.5 w-2.5 rounded-sm ${cls}`} aria-hidden="true" />
      {label}
    </span>
  );
}

function ReviewHeader({
  view,
  ready,
  title,
  onRun,
  running,
  building,
  error,
}: {
  view: Built;
  ready: number;
  title: string;
  onRun: () => void;
  running: boolean;
  building?: boolean;
  error?: string | null;
}) {
  const now = useNow(30_000);
  const vol = useStore((s) => s.status?.volumes.find((v) => samePath(v.root, view.header.root)) ?? selectedVolume(s.status?.volumes));
  const lastRefresh = useStore((s) => s.status?.index.last_refresh_ms);
  const free = view.header.free_bytes ?? vol?.free_bytes ?? 0;
  const used = view.header.used_bytes ?? (vol ? vol.total_bytes - vol.free_bytes : 0);
  const folder = view.review.scope_path;
  return (
    <section aria-label={title} className="fade-up rounded-2xl border border-line bg-ink-850 p-4">
      <div className="flex flex-wrap items-start gap-x-4 gap-y-3">
        <div className="min-w-0 flex-1">
          <div className="label truncate text-[length:var(--fs-xs)] uppercase">{title}</div>
          {folder && (
            <div className="truncate font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold text-paper" title={folder}>
              {folder}
            </div>
          )}
          <div className="mt-1 flex flex-wrap items-baseline gap-x-2">
            <span className="font-[family-name:var(--font-display)] text-[30px] font-extrabold leading-none tracking-tight text-brass tabular-nums" data-ready={ready}>
              {humanBytes(ready)}
            </span>
            <span className="font-semibold text-paper-dim">ready to free</span>
          </div>
        </div>
        <button
          type="button"
          className="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-line bg-ink-900 px-3 py-1.5 text-[length:var(--fs-sm)] font-bold text-paper-dim hover:border-brass/60 hover:text-brass disabled:opacity-50"
          disabled={running}
          title={folder ? `Review ${folder} again` : `Review ${drive(view.header.root)} again now`}
          onClick={onRun}
        >
          {running ? <SpinnerIcon /> : <RefreshIcon />}
          {running ? "Reviewing…" : folder ? "Review again" : "Review now"}
        </button>
      </div>
      {!folder && <SpaceBar used={used} free={free} ready={ready} />}
      <div className="mt-2 text-[length:var(--fs-xs)] text-faint">
        Built {humanAge(now - view.review.created_ms)}
        {lastRefresh ? ` · index from ${humanAge(now - lastRefresh)}` : ""}
      </div>
      {building && (
        <p role="status" className="mt-2 flex items-center gap-2 rounded-xl border border-line-soft bg-ink-900 px-3 py-1.5 text-[length:var(--fs-sm)] text-paper-dim">
          <span className="shrink-0 text-brass">
            <SpinnerIcon />
          </span>
          <span>
            <span className="font-semibold text-paper">Building a newer review…</span> This one stays until it is ready.
          </span>
        </p>
      )}
      {error && <p className="mt-1 text-[length:var(--fs-sm)] text-err">{error}</p>}
    </section>
  );
}

function BuildingHeader({ view, title }: { view: Review; title: string }) {
  const vol = useStore((s) => s.status?.volumes.find((v) => samePath(v.root, view.header.root)));
  const free = view.header.free_bytes ?? vol?.free_bytes ?? 0;
  const used = view.header.used_bytes ?? (vol ? vol.total_bytes - vol.free_bytes : 0);
  return (
    <section aria-label={title} aria-busy="true" className="fade-up rounded-2xl border border-line bg-ink-850 p-4">
      <div className="label truncate text-[length:var(--fs-xs)] uppercase">{title}</div>
      <div className="mt-1 flex items-center gap-2.5">
        <span className="shrink-0 text-brass">
          <SpinnerIcon size={22} />
        </span>
        <span className="font-[family-name:var(--font-display)] text-[24px] font-extrabold leading-tight tracking-tight text-paper">Building your review…</span>
      </div>
      <p className="mt-1 text-[length:var(--fs-sm)] text-muted">
        Hoardini is going over {drive(view.header.root)} for what can go and what is big but stays. Nothing is changed while it looks.
      </p>
      <SpaceBar used={used} free={free} ready={null} />
    </section>
  );
}

function Cards({ cards, view }: { cards: ReviewCardRef[]; view: Built }) {
  const proposals = useStore((s) => s.proposals);
  const refs = useMemo(() => new Map(cards.map((c) => [c.proposal_id, c])), [cards]);
  // Order once per review, so cards do not jump while ticks change or a job runs.
  const ordered = useRef<{ key: string; ids: string[] }>({ key: "", ids: [] });
  const key = `${view.review.id}|${cards.map((c) => c.proposal_id).join(",")}`;
  if (ordered.current.key !== key) ordered.current = { key, ids: orderCards(cards, proposals).map((c) => c.proposal_id) };
  return (
    <div>
      {ordered.current.ids.map((id) => (
        <ProposalCard key={id} proposalId={id} title={refs.get(id)?.title} note={refs.get(id)?.carried ? "from the earlier review" : undefined} />
      ))}
    </div>
  );
}

function Empty({ title, used, free }: { title: string; used: number | null; free: number | null }) {
  return (
    <div className="fade-up rounded-2xl border border-dashed border-line px-4 py-5 text-center">
      <div className="font-[family-name:var(--font-display)] text-[length:var(--fs-md)] font-bold text-paper">{title}</div>
      <p className="mt-1 text-[length:var(--fs-sm)] text-muted">
        {used != null && free != null ? `${humanBytes(used)} used and ${humanBytes(free)} free. ` : ""}
        Hoardini looks again when the index changes, or when you press Review now.
      </p>
    </div>
  );
}

const isPending = (p: Proposal | undefined) => p?.status === "pending" || p?.status === "approved" || p?.status === "running";

function useShownCards(): ReviewCardRef[] {
  const main = useReview((s) => s.main);
  const carry = useReview((s) => s.carry);
  const proposals = useStore((s) => s.proposals);
  return useMemo(() => {
    if (!main) return [];
    // A replaced card stays only while it shows something: the user's ticks, a job, or the replaced line.
    const kept = carry.filter((c) => proposals[c.proposal_id] && (proposals[c.proposal_id]!.status !== "pending" || proposals[c.proposal_id]!.touched));
    return [...kept, ...main.cards];
  }, [main, carry, proposals]);
}

export const useDriveReady = () => useReady(useShownCards());

function DriveReview() {
  const { main, bare, status, error, running, runNow, markAsked } = useReview();
  const building = useBuilding();
  const proposals = useStore((s) => s.proposals);
  const shown = useShownCards();
  const ready = useReady(shown);

  if (status === "no-index") return <Hello />;
  if (!main && bare) {
    const title = `Review of ${drive(bare.header.root)}`;
    return (
      <>
        <BuildingHeader view={bare} title={title} />
        <OnlineChecksConsent online={bare.online} onAnswered={markAsked} />
        <RarelyUsedApps given={bare.apps} />
      </>
    );
  }
  if (!main) {
    if (status === "error")
      return (
        <div className="rounded-2xl border border-err/30 bg-err/10 p-4 text-[length:var(--fs-sm)] text-err">
          Could not build the review: {error}
          <button type="button" className="ml-2 font-bold underline underline-offset-2" onClick={() => void useReview.getState().open()}>
            Try again
          </button>
        </div>
      );
    return (
      <div className="rounded-2xl border border-line bg-ink-850 p-4 text-muted">
        <span className="pulse-soft">Looking over the drive…</span>
      </div>
    );
  }
  const where = drive(main.header.root);
  const live = shown.filter((c) => proposals[c.proposal_id]?.status !== "superseded");
  return (
    <>
      <ReviewHeader view={main} ready={ready} title={`Review of ${where}`} onRun={() => void runNow()} running={running} building={building} error={error} />
      <OnlineChecksConsent online={main.online} onAnswered={markAsked} />
      <section aria-label="What can go">
        <SectionHead title="What can go" count={live.length}>
          Each card is approved on its own. Nothing happens until you approve it.
        </SectionHead>
        {live.length === 0 && <Empty title={`Nothing new to clean on ${where}`} used={main.header.used_bytes} free={main.header.free_bytes} />}
        <Cards cards={shown} view={main} />
      </section>
      {!main.review.scope_path && <RarelyUsedApps given={main.apps} />}
      <LargeAreas areas={main.areas} scope={where} />
    </>
  );
}

function FolderReview() {
  const folder = useReview((s) => s.folder)!;
  const main = useReview((s) => s.main);
  const { cleanUp, closeFolder, markAsked } = useReview.getState();
  const proposals = useStore((s) => s.proposals);
  const view = folder.view;
  const cards = view?.cards ?? [];
  const ready = useReady(cards);
  const already = useMemo(
    () =>
      (main?.cards ?? []).filter((c) => {
        const p = proposals[c.proposal_id];
        return isPending(p) && p!.items.some((i) => samePath(i.path, folder.path) || isUnder(folder.path, i.path) || isUnder(i.path, folder.path));
      }),
    [main, proposals, folder.path],
  );
  const back = main ? drive(main.header.root) : "the drive";
  return (
    <>
      <button type="button" className="inline-flex items-center gap-1 text-[length:var(--fs-sm)] font-bold text-brass hover:underline" onClick={closeFolder}>
        <span aria-hidden="true">←</span> Back to the {back} review
      </button>
      {!view &&
        (folder.error ? (
          <div className="rounded-2xl border border-err/30 bg-err/10 p-4 text-[length:var(--fs-sm)] text-err">Could not review {folder.path}: {folder.error}</div>
        ) : (
          <div className="rounded-2xl border border-line bg-ink-850 p-4 text-muted">
            <span className="pulse-soft">Looking over {folder.path}…</span>
          </div>
        ))}
      {view && (
        <>
          <ReviewHeader view={view} ready={ready} title="Clean up" onRun={() => void cleanUp(folder.path)} running={false} />
          <OnlineChecksConsent online={view.online} onAnswered={markAsked} />
          <section aria-label="What can go">
            <SectionHead title="What can go" count={cards.length} />
            {cards.length === 0 && (
              <Empty title={`Nothing new to clean in ${folder.path}`} used={null} free={null} />
            )}
            <Cards cards={cards} view={view} />
          </section>
          {already.length > 0 && (
            <section aria-label={`Already in the ${back} review`}>
              <SectionHead title={`Already in the ${back} review`} count={already.length}>
                These cards hold items in {folder.path}, so this review leaves them out.
              </SectionHead>
              {already.map((c) => (
                <ProposalCard key={c.proposal_id} proposalId={c.proposal_id} title={c.title} />
              ))}
            </section>
          )}
          <RarelyUsedApps given={view.apps ?? []} />
          <LargeAreas areas={view.areas} scope={folder.path} />
        </>
      )}
    </>
  );
}

export function ReviewView({ hidden }: { hidden: boolean }) {
  useReviewSync();
  const folder = useReview((s) => s.folder);
  const whatIs = useReview((s) => s.whatIs);
  const scroller = useRef<HTMLDivElement>(null);
  useEffect(() => {
    scroller.current?.scrollTo?.({ top: 0 });
  }, [folder?.path, whatIs]);
  return (
    <div ref={scroller} className={`${hidden ? "hidden" : ""} min-h-0 flex-1 overflow-y-auto overflow-x-hidden`} role="region" aria-label="Review">
      <div className="@container mx-auto w-full max-w-4xl space-y-5 px-4 pb-10 pt-4">
        {whatIs && <WhatIsCard key={whatIs} path={whatIs} />}
        {folder ? <FolderReview /> : <DriveReview />}
      </div>
    </div>
  );
}
