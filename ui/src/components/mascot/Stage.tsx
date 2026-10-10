import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import { routes } from "../../api/client";
import type { Job, Proposal } from "../../api/types";
import { fileName, humanBytes } from "../../format";
import { useStore } from "../../store";
import { SumoSvg } from "../Mascot";
import { ACT_MS, FIN_ACT, FIN_BASE_MS, WORK_ACT, girthLevel, moveFor, reducedMotion, type Act, type Move, type OneShot } from "./acts";
import { cueHeader, holdGirth, releaseGirth, useMascotStage } from "./cue";
import { girthFor, selectedVolume, usedFraction } from "./girth";

const CLIMB_MS = 1600;
const BACK_MS = 1400;
const WORK_MIN_MS = 2000;
const LOST_MS = 900;
const HURRY_MS = 380;
const LONG_MS = 30_000;
const REPEAT_WINDOW_MS = 10 * 60_000;
const REPEAT_MAX = 5;
const PW = 60;
const PH = 34;
const LW = 20;
const BW = 46;
const BH = 40;
const CW = 74;
const CH = 46;
const FOOT = 61.4 / 64;
const INK = "#4a3a52";

type Outcome = "done" | "failed" | "cancelled";
type Phase = "down" | "work" | "finish" | "up" | "gone";
type Pt = { x: number; y: number };

interface Parcel {
  name: string;
  size: string;
  folder: boolean;
  count: number;
}
interface Layout {
  S: number;
  sx: number;
  sy: number;
  ground: number;
  home: Pt;
  ladder: { x: number; top: number; height: number };
  parcels: Pt[];
  box: Pt;
  cloud: Pt;
}
interface View {
  seq: number;
  phase: Phase;
  act: Act;
  move: Move;
  parcel: Parcel;
  layout: Layout;
  sag: number;
  prop: "box" | "cloud" | null;
}
interface Trip {
  seq: number;
  jobId: string;
  proposalId: string;
  move: Move;
  phase: Phase;
  outcome: Outcome | null;
  fast: boolean;
  startedAt: number;
  workAt: number;
  lostAt?: number;
  layout: Layout;
  scroller: Element | null;
}
interface Els {
  sumo: RefObject<HTMLDivElement | null>;
  parcels: RefObject<(HTMLDivElement | null)[]>;
  prop: RefObject<HTMLDivElement | null>;
  flaps: RefObject<(SVGGElement | null)[]>;
  flash: RefObject<HTMLDivElement | null>;
}

// Review and chat cards stay mounted behind hidden tabs; only one on screen can carry him.
function cardEl(id: string): HTMLElement | null {
  const all = [...document.querySelectorAll<HTMLElement>(`[data-proposal-id="${CSS.escape(id)}"]`)];
  return all.find((el) => el.getClientRects().length > 0) ?? all[0] ?? null;
}

function scrollerOf(el: Element): Element | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const o = getComputedStyle(p).overflowY;
    if (o === "auto" || o === "scroll") return p;
  }
  return null;
}

const inView = (y: number, view: { top: number; bottom: number }) => y >= view.top + 8 && y <= Math.min(view.bottom, innerHeight) - 40;

// He stands on the action footer where the user just clicked; a collapsed parent hands him to its follow-up card.
function groundCandidates(card: HTMLElement): { el: HTMLElement; y: number }[] {
  const out: { el: HTMLElement; y: number }[] = [];
  const collapsed = card.dataset.collapsed != null;
  const follow = card.dataset.followUp ? cardEl(card.dataset.followUp) : null;
  if (collapsed && follow) out.push({ el: follow, y: follow.getBoundingClientRect().top });
  const foot = card.querySelector<HTMLElement>("[data-proposal-foot]");
  if (foot && !collapsed) out.push({ el: card, y: foot.getBoundingClientRect().top });
  out.push({ el: card, y: card.getBoundingClientRect().top });
  return out;
}

// Null when neither the footer nor the card's top edge is comfortably on screen.
function layoutFor(card: HTMLElement, move: Move, scroller: Element | null): Layout | null {
  const homeEl = document.querySelector<HTMLElement>("[data-mascot-home]");
  if (!homeEl) return null;
  const h = homeEl.getBoundingClientRect();
  const view = scroller?.getBoundingClientRect() ?? { top: 0, bottom: innerHeight };
  const at = groundCandidates(card).find((g) => inView(g.y, view));
  if (!at || h.width === 0) return null;
  const c = at.el.getBoundingClientRect();
  if (c.width < h.width + PW * 2 + 40) return null;
  const S = h.width;
  const twin = move === "clap";
  const sx = c.left + 8 + (twin ? PW + 6 : 0);
  const ground = at.y + 1;
  const sy = ground - S * FOOT;
  const py = ground - PH;
  const parcels = twin ? [{ x: c.left + 8, y: py }, { x: sx + S + 4, y: py }] : [{ x: sx + S + 2, y: py }];
  const last = parcels[parcels.length - 1]!;
  const ladderTop = h.bottom - 6;
  return {
    S,
    sx,
    sy,
    ground,
    home: { x: h.left - sx, y: h.top - sy },
    ladder: { x: sx + S / 2 - LW / 2, top: ladderTop, height: Math.max(0, ground - ladderTop) },
    parcels,
    box: { x: Math.min(c.right - BW - 8, last.x + PW + 64), y: ground - BH },
    cloud: { x: Math.min(innerWidth - CW - 8, last.x + PW + 40), y: Math.max(8, ground - 150) },
  };
}

function parcelFor(p: Proposal, job: Job): Parcel {
  const items = p.items.filter((i) => i.selected && !i.block_reason && i.status !== "blocked");
  const first = items[0];
  const keep = p.kind === "dedupe" ? p.dedupe?.keep : undefined;
  const bytes = job.progress.total_bytes || items.reduce((a, i) => a + i.alloc, 0) || p.total_alloc;
  return {
    name: keep ? fileName(keep) : first ? fileName(first.path) : p.kind,
    size: humanBytes(bytes),
    folder: !keep && first?.kind === "dir",
    count: keep ? 1 : items.length,
  };
}

function move(el: Element | null | undefined, frames: Keyframe[], opts: KeyframeAnimationOptions): Animation | null {
  if (!el) return null;
  const cs = getComputedStyle(el);
  const from = { transform: cs.transform, opacity: cs.opacity };
  el.getAnimations().forEach((a) => a.cancel());
  return el.animate([from, ...frames], { fill: "both", ...opts });
}

function actFor(t: Trip): Act {
  switch (t.phase) {
    case "work":
      return WORK_ACT[t.move];
    case "finish":
      return t.outcome === "done" ? FIN_ACT[t.move] : t.outcome === "failed" ? "slip" : "shrug";
    default:
      return "climb";
  }
}

class Director {
  trip: Trip | null = null;
  private headerJobs = new Map<string, { move: Move; at: number }>();
  private tripLog: number[] = [];
  private known = new Map<string, Job["status"]>();
  private afterHome: (() => void)[] = [];
  private seq = 0;
  private timers = new Set<ReturnType<typeof setTimeout>>();
  private frame = 0;
  private listening = false;
  // Cards collapse and follow-ups appear mid-trip without any scroll event.
  private poll: ReturnType<typeof setInterval> | undefined;

  constructor(
    private setView: (fn: (v: View | null) => View | null) => void,
    private els: Els,
  ) {}

  attach(): () => void {
    for (const j of Object.values(useStore.getState().jobs)) this.known.set(j.id, j.status);
    const unsub = useStore.subscribe((s, prev) => {
      if (s.jobs !== prev.jobs) this.onJobs(s.jobs);
      if (s.proposals !== prev.proposals) this.onProposals(s.proposals, prev.proposals);
    });
    return () => {
      unsub();
      this.timers.forEach(clearTimeout);
      this.timers.clear();
      this.unlisten();
      if (this.trip) useStore.getState().setMascotAway(false);
      this.trip = null;
      this.setView(() => null);
    };
  }

  private later(ms: number, fn: () => void, phase?: Phase) {
    const seq = this.trip?.seq;
    const t = setTimeout(() => {
      this.timers.delete(t);
      if (this.trip?.seq !== seq || (phase && this.trip?.phase !== phase)) return;
      fn();
    }, ms);
    this.timers.add(t);
  }

  private onJobs(jobs: Record<string, Job>) {
    for (const j of Object.values(jobs)) {
      const was = this.known.get(j.id);
      if (was === j.status) continue;
      this.known.set(j.id, j.status);
      if (!j.proposal_id) continue;
      const ended = j.status === "done" || j.status === "failed" || j.status === "cancelled";
      // A fast job can arrive already finished; old finished jobs from a reconnect are ignored.
      const fresh = ended && was === undefined && j.finished_ms != null && Date.now() - j.finished_ms < 10_000;
      if (j.status === "running" || fresh) this.start(j);
      if (ended) this.end(j.id, j.status as Outcome);
    }
  }

  private onProposals(next: Record<string, Proposal>, prev: Record<string, Proposal>) {
    for (const [id, p] of Object.entries(next)) {
      if (p.status !== "declined" || prev[id]?.status !== "pending") continue;
      if (!reducedMotion() && !useStore.getState().mascotAway) cueHeader("shake");
    }
  }

  private canTrip(): boolean {
    const now = Date.now();
    this.tripLog = this.tripLog.filter((t) => now - t < REPEAT_WINDOW_MS);
    return !reducedMotion() && !this.trip && !document.hidden && !useStore.getState().mascotAway && this.tripLog.length < REPEAT_MAX;
  }

  private start(j: Job) {
    if (this.trip?.jobId === j.id || this.headerJobs.has(j.id)) return;
    const st = useStore.getState();
    const p = st.proposals[j.proposal_id!];
    const mv = moveFor(j.kind || p?.kind || "");
    if (!mv) return;
    holdGirth(selectedVolume(st.status?.volumes));
    if (mv === "throw") void this.aimUp(j.id, j.proposal_id!);
    const card = p && this.canTrip() ? cardEl(p.id) : null;
    const scroller = card && scrollerOf(card);
    const layout = card && layoutFor(card, mv, scroller ?? null);
    if (p && layout) return this.beginTrip(j, p, mv, layout, scroller ?? null);
    this.headerJobs.set(j.id, { move: mv, at: Date.now() });
    this.syncHeaderWork();
  }

  private async aimUp(jobId: string, proposalId: string) {
    const st = useStore.getState();
    const p = st.proposals[proposalId]?.destination ? st.proposals[proposalId] : await st.loadProposal(proposalId);
    const dests = p?.destination ? await routes.destinations().catch(() => []) : [];
    if (dests.find((d) => d.id === p?.destination?.id)?.kind !== "rclone") return;
    if (this.trip?.jobId === jobId && this.trip.phase !== "finish") this.trip.move = "throw-up";
    const h = this.headerJobs.get(jobId);
    if (h) h.move = "throw-up";
  }

  private beginTrip(j: Job, p: Proposal, mv: Move, layout: Layout, scroller: Element | null) {
    this.tripLog.push(Date.now());
    const seq = ++this.seq;
    this.trip = { seq, jobId: j.id, proposalId: p.id, move: mv, phase: "down", outcome: null, fast: false, startedAt: Date.now(), workAt: 0, layout, scroller };
    const g = girthLevel(girthFor(usedFraction(useMascotStage.getState().hold ?? selectedVolume(useStore.getState().status?.volumes)) ?? 0.5));
    useStore.getState().setMascotAway(true);
    this.setView(() => ({ seq, phase: "down", act: "climb", move: mv, parcel: parcelFor(p, j), layout, sag: g > 0.45 ? (g - 0.45) * 7 : 0, prop: null }));
    this.listen();
  }

  private end(jobId: string, outcome: Outcome) {
    const t = this.trip;
    if (t?.jobId === jobId && !this.headerJobs.has(jobId)) {
      t.outcome = outcome;
      if (t.phase === "work") this.finishWhenReady(t);
      return;
    }
    const h = this.headerJobs.get(jobId);
    if (!h) return;
    const play = () => {
      this.headerJobs.delete(jobId);
      this.syncHeaderWork();
      const fin = () => this.headerFinish(h.move, outcome);
      if (useStore.getState().mascotAway) this.afterHome.push(fin);
      else fin();
    };
    const wait = h.at + WORK_MIN_MS - Date.now();
    if (wait <= 0) return play();
    const timer = setTimeout(() => {
      this.timers.delete(timer);
      play();
    }, wait);
    this.timers.add(timer);
  }

  private finishWhenReady(t: Trip) {
    const wait = t.workAt + WORK_MIN_MS - Date.now();
    if (wait <= 0) this.go("finish");
    else this.later(wait, () => this.go("finish"), "work");
  }

  private headerFinish(mv: Move, outcome: Outcome) {
    if (reducedMotion()) {
      releaseGirth();
      if (outcome === "done") this.pleased();
      return;
    }
    const act: OneShot = outcome === "done" ? FIN_ACT[mv] : outcome === "failed" ? "slip" : "shrug";
    cueHeader(act);
    const t = setTimeout(() => {
      this.timers.delete(t);
      releaseGirth();
    }, ACT_MS[act]);
    this.timers.add(t);
  }

  private pleased() {
    useMascotStage.setState({ quietTadaUntil: useStore.getState().tadaUntil + 100 });
    cueHeader("pleased");
  }

  private syncHeaderWork() {
    useMascotStage.setState({ headerWork: this.headerJobs.size > 0 });
  }

  private hurry = () => {
    const t = this.trip;
    if (!t || t.phase === "up" || t.phase === "gone") return;
    if (!t.outcome) {
      this.headerJobs.set(t.jobId, { move: t.move, at: t.startedAt });
      this.syncHeaderWork();
    }
    t.fast = true;
    this.go("up");
  };

  private onScroll = () => {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      const t = this.trip;
      if (!t || t.phase === "gone") return;
      const card = cardEl(t.proposalId);
      const layout = card && layoutFor(card, t.move, t.scroller);
      if (!layout) {
        t.lostAt ??= Date.now();
        if (t.phase !== "up" && Date.now() - t.lostAt > LOST_MS) this.hurry();
        return;
      }
      t.lostAt = undefined;
      const o = t.layout;
      if (o.ground === layout.ground && o.sx === layout.sx && o.home.x === layout.home.x && o.home.y === layout.home.y) return;
      t.layout = layout;
      this.setView((v) => (v?.seq === t.seq ? { ...v, layout } : v));
    });
  };

  private listen() {
    if (this.listening) return;
    this.listening = true;
    window.addEventListener("scroll", this.onScroll, { capture: true, passive: true });
    window.addEventListener("resize", this.onScroll, { passive: true });
    window.addEventListener("pointerdown", this.hurry, { capture: true, passive: true });
    this.poll = setInterval(this.onScroll, 250);
  }

  private unlisten() {
    if (!this.listening) return;
    this.listening = false;
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    clearInterval(this.poll);
    window.removeEventListener("scroll", this.onScroll, { capture: true });
    window.removeEventListener("resize", this.onScroll);
    window.removeEventListener("pointerdown", this.hurry, { capture: true });
  }

  private go(phase: Phase) {
    const t = this.trip;
    if (!t) return;
    t.phase = phase;
    const act = actFor(t);
    const prop = phase === "finish" && t.outcome === "done" ? (t.move === "throw" ? "box" : t.move === "throw-up" ? "cloud" : null) : undefined;
    this.setView((v) => (v?.seq === t.seq ? { ...v, phase, act, move: t.move, prop: prop === undefined ? v.prop : prop } : v));
  }

  // Called after React commits a phase, so every element of the scene exists.
  run(v: View) {
    const t = this.trip;
    if (!t || t.seq !== v.seq || t.phase !== v.phase) return;
    const L = t.layout;
    const sumo = this.els.sumo.current;
    const parcels = this.els.parcels.current ?? [];
    switch (v.phase) {
      case "down": {
        sumo?.animate(
          [{ transform: `translate(${L.home.x}px, ${L.home.y}px)` }, { transform: `translate(0px, ${L.home.y}px)`, offset: 0.18 }, { transform: "none" }],
          { duration: CLIMB_MS, easing: "linear", fill: "both" },
        );
        parcels.forEach((el, i) =>
          el?.animate(
            [
              { transform: "translateY(22px) scale(0.3)", opacity: 0 },
              { transform: "translateY(-12px) scale(1.05)", opacity: 1, offset: 0.55 },
              { transform: "none", opacity: 1 },
            ],
            { duration: 420, delay: CLIMB_MS - 320 + i * 70, easing: "ease-out", fill: "backwards" },
          ),
        );
        this.later(CLIMB_MS, () => this.go("work"), "down");
        return;
      }
      case "work": {
        t.workAt = Date.now();
        if (t.outcome) this.finishWhenReady(t);
        if (t.move === "flop") {
          const o = this.seat(L);
          move(sumo, [{ transform: `translate(${o.x / 2}px, ${o.y - 18}px)`, offset: 0.5 }, { transform: `translate(${o.x}px, ${o.y}px)` }], { duration: 420, easing: "ease-in-out" });
        }
        this.later(
          LONG_MS,
          () => {
            this.headerJobs.set(t.jobId, { move: t.move, at: t.startedAt });
            this.syncHeaderWork();
            this.go("up");
          },
          "work",
        );
        return;
      }
      case "finish": {
        const act = v.act as OneShot;
        if (t.outcome === "done") this.finisher(t, L);
        else {
          move(sumo, [{ transform: "none" }], { duration: 240, easing: "ease-out" });
          if (t.outcome === "cancelled") this.parcelsHome(150);
        }
        this.later(ACT_MS[act], () => this.go("up"), "finish");
        return;
      }
      case "up": {
        if (t.outcome) releaseGirth();
        const ms = t.fast ? HURRY_MS : BACK_MS;
        move(sumo, [{ transform: `translate(0px, ${L.home.y}px)`, offset: 0.82 }, { transform: `translate(${L.home.x}px, ${L.home.y}px)` }], { duration: ms, easing: "linear" });
        if (!(t.outcome === "done" && (t.move === "push" || t.move === "throw" || t.move === "throw-up"))) this.parcelsHome(0);
        this.els.prop.current?.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 350, delay: 250, fill: "forwards" });
        this.later(ms, () => this.arrive(), "up");
        return;
      }
      case "gone":
        return;
    }
  }

  private seat(L: Layout): Pt {
    const p = L.parcels[0]!;
    return { x: p.x + PW / 2 - (L.sx + L.S / 2), y: -PH + 2 + L.S * (FOOT - 56 / 64) };
  }

  private parcelsHome(delay: number) {
    for (const el of this.els.parcels.current ?? []) {
      if (!el || Number(getComputedStyle(el).opacity) < 0.05) continue;
      const m = new DOMMatrixReadOnly(getComputedStyle(el).transform);
      move(el, [{ transform: `translate(${m.e}px, ${Math.min(m.f, 0) - 14}px) scale(${m.a}, ${m.d})`, offset: 0.4 }, { transform: `translate(${m.e}px, 24px) scale(0.3)`, opacity: 0 }], {
        duration: 440,
        delay,
        easing: "ease-in",
      });
    }
  }

  private finisher(t: Trip, L: Layout) {
    const sumo = this.els.sumo.current;
    const [a, b] = this.els.parcels.current ?? [];
    const P = L.parcels[0]!;
    const k = ACT_MS[FIN_ACT[t.move]] / FIN_BASE_MS[t.move];
    switch (t.move) {
      case "push": {
        const dx = innerWidth + 40 - P.x;
        move(sumo, [{ transform: "translateX(-6px)", offset: 0.22 }, { transform: "translateX(26px)", offset: 0.42 }, { transform: "translateX(20px)", offset: 0.55 }, { transform: "none" }], {
          duration: 1100 * k,
          easing: "ease-in-out",
        });
        move(a, [{ transform: `translate(${dx * 0.45}px, -48px) rotate(420deg)`, offset: 0.45 }, { transform: `translate(${dx}px, -6px) rotate(960deg)` }], {
          duration: 760 * k,
          delay: 460 * k,
          easing: "cubic-bezier(0.25, 0.1, 0.6, 1)",
        });
        return;
      }
      case "flop": {
        const o = this.seat(L);
        move(
          sumo,
          [
            { transform: `translate(${o.x * 0.8}px, ${o.y - 34}px)`, offset: 0.38 },
            { transform: `translate(${o.x}px, ${o.y + 4}px)`, offset: 0.55 },
            { transform: `translate(${o.x}px, ${o.y + PH * 0.68}px)`, offset: 0.64 },
            { transform: `translate(${o.x * 0.4}px, -16px)`, offset: 0.84 },
            { transform: "none" },
          ],
          { duration: 1300 * k, easing: "ease-in-out" },
        );
        move(a, [{ transform: `translateY(${PH * 0.35}px) scale(1.45, 0.3)` }], { duration: 110 * k, delay: 715 * k, easing: "ease-out" });
        return;
      }
      case "throw":
      case "throw-up": {
        const up = t.move === "throw-up";
        const hand = { x: L.sx + L.S * 0.72 - PW / 2 - P.x, y: L.sy - PH * 0.6 - P.y };
        const dest = up ? { x: L.cloud.x + CW / 2 - PW / 2 - P.x, y: L.cloud.y + CH / 2 - PH / 2 - P.y } : { x: L.box.x + BW / 2 - PW / 2 - P.x, y: L.box.y - 6 - P.y };
        const peak = Math.min(hand.y, dest.y) - (up ? 30 : 50);
        move(
          a,
          [
            { transform: `translate(${hand.x}px, ${hand.y}px) scale(0.9)`, offset: 0.22 },
            { transform: `translate(${hand.x}px, ${hand.y}px) scale(0.9)`, offset: 0.38 },
            { transform: `translate(${(hand.x + dest.x) / 2}px, ${peak}px) rotate(200deg) scale(0.8)`, offset: 0.6 },
            { transform: `translate(${dest.x}px, ${dest.y}px) rotate(360deg) scale(0.6)`, opacity: 1, offset: 0.78 },
            { transform: `translate(${dest.x}px, ${dest.y + (up ? 0 : 16)}px) rotate(360deg) scale(${up ? 0.3 : 0.5})`, opacity: 0, offset: 0.88 },
            { transform: `translate(${dest.x}px, ${dest.y + (up ? 0 : 16)}px) scale(0.3)`, opacity: 0 },
          ],
          { duration: 1300 * k, easing: "linear" },
        );
        if (up) {
          this.els.prop.current?.animate([{ transform: "none" }, { transform: "scale(1.18)" }, { transform: "none" }], { duration: 380 * k, delay: 1000 * k, easing: "ease-out" });
        } else {
          for (const f of this.els.flaps.current ?? []) {
            const from = getComputedStyle(f!).transform;
            f?.animate([{ transform: from }, { transform: "rotate(0deg)" }], { duration: 220 * k, delay: 1080 * k, fill: "forwards", easing: "cubic-bezier(0.3, 1.5, 0.6, 1)" });
          }
        }
        return;
      }
      case "clap": {
        const mx = L.sx + L.S / 2 - PW / 2;
        const pa = L.parcels[0]!;
        const pb = L.parcels[1] ?? pa;
        move(
          a,
          [
            { transform: `translate(${(mx - pa.x) * 0.2}px, -6px)`, offset: 0.15 },
            { transform: `translate(${mx - pa.x}px, 0px) scale(0.8)`, opacity: 1, offset: 0.33 },
            { transform: `translate(${mx - pa.x}px, 0px) scale(0.8)`, opacity: 0, offset: 0.4 },
            { transform: `translate(${mx - pa.x}px, 0px) scale(0.8)`, opacity: 0 },
          ],
          { duration: 1100 * k, easing: "ease-in" },
        );
        move(
          b,
          [
            { transform: `translate(${(mx - pb.x) * 0.2}px, -6px)`, offset: 0.15 },
            { transform: `translate(${mx - pb.x}px, 0px) scale(0.8)`, offset: 0.33 },
            { transform: `translate(${mx - pb.x}px, -3px) scale(1)`, offset: 0.42 },
            { transform: `translate(${mx - pb.x}px, ${-L.S * 0.95}px) scale(0.85)`, offset: 0.62 },
            { transform: `translate(${mx - pb.x}px, ${-L.S * 0.9}px) scale(0.85)` },
          ],
          { duration: 1100 * k, easing: "ease-in" },
        );
        this.els.flash.current?.animate(
          [
            { transform: "scale(0.2)", opacity: 0 },
            { transform: "scale(0.3)", opacity: 1, offset: 0.05 },
            { transform: "scale(1.8)", opacity: 0 },
          ],
          { duration: 420 * k, delay: 360 * k, fill: "both", easing: "ease-out" },
        );
        return;
      }
    }
  }

  private arrive() {
    const t = this.trip;
    if (!t) return;
    t.phase = "gone";
    this.unlisten();
    useStore.getState().setMascotAway(false);
    this.setView((v) => (v?.seq === t.seq ? { ...v, phase: "gone" } : v));
    if (t.outcome === "done") this.pleased();
    for (const f of this.afterHome.splice(0)) f();
    this.later(240, () => {
      this.trip = null;
      this.setView(() => null);
    });
  }
}

function ParcelArt({ p }: { p: Parcel }) {
  return (
    <>
      <svg viewBox="0 0 12 12" className="m-parcel-icon" aria-hidden>
        {p.folder ? (
          <path d="M1 2.6h3.6l1.1 1.3H11v6.5H1z" fill="#f2d48a" stroke={INK} strokeWidth="1" strokeLinejoin="round" />
        ) : (
          <path d="M2.5 1h4.6L9.6 3.5V11H2.5zM7.1 1v2.5h2.5" fill="#fffdf8" stroke={INK} strokeWidth="1" strokeLinejoin="round" />
        )}
      </svg>
      <span className="m-parcel-name">{p.name}</span>
      <span className="m-parcel-size">{p.size}</span>
      {p.count > 1 && <span className="m-parcel-count">×{p.count}</span>}
    </>
  );
}

function Ladder({ height, sag, out }: { height: number; sag: number; out: boolean }) {
  const rungs = Array.from({ length: Math.max(0, Math.floor((height - 4) / 9)) }, (_, i) => 6 + i * 9);
  return (
    <svg width={LW} height={Math.max(1, height)} className={`m-ladder${out ? " m-ladder-out" : ""}`} overflow="visible" aria-hidden>
      <g stroke="#9a6a3c" strokeLinecap="round" fill="none">
        <path d={`M3 0V${height}M${LW - 3} 0V${height}`} strokeWidth="2.6" />
        {rungs.map((y) => (
          <path
            key={y}
            className={sag > 0 ? "m-rung m-bend" : "m-rung"}
            style={{ "--d": `${Math.round(CLIMB_MS * (0.18 + (0.82 * y) / Math.max(1, height)))}ms` } as CSSProperties}
            d={`M3 ${y}Q${LW / 2} ${y + sag * 2} ${LW - 3} ${y}`}
            strokeWidth="1.8"
            vectorEffect="non-scaling-stroke"
          />
        ))}
      </g>
    </svg>
  );
}

function BoxArt({ flaps }: { flaps: RefObject<(SVGGElement | null)[]> }) {
  return (
    <svg width={BW} height={BH} viewBox={`0 0 ${BW} ${BH}`} overflow="visible" aria-hidden>
      <g ref={(el) => void (flaps.current[0] = el)} className="m-flap" style={{ transformOrigin: "4px 12px", transform: "rotate(-128deg)" }}>
        <rect x="4" y="10" width="19" height="4" rx="1" fill="#dba86a" stroke={INK} strokeWidth="1.2" />
      </g>
      <g ref={(el) => void (flaps.current[1] = el)} className="m-flap" style={{ transformOrigin: "42px 12px", transform: "rotate(128deg)" }}>
        <rect x="23" y="10" width="19" height="4" rx="1" fill="#dba86a" stroke={INK} strokeWidth="1.2" />
      </g>
      <rect x="4" y="12" width="38" height="27" rx="2" fill="#c98f55" stroke={INK} strokeWidth="1.3" />
      <rect x="19.5" y="12.6" width="7" height="9" fill="#ecd3a6" opacity="0.85" />
      <path d="M9 31h9" stroke={INK} strokeWidth="1" strokeLinecap="round" opacity="0.45" />
    </svg>
  );
}

function CloudArt() {
  return (
    <svg width={CW} height={CH} viewBox="0 0 74 46" overflow="visible" aria-hidden>
      <path
        d="M17 42C7 42 5 29 14.5 27.5C13 16 27 10 34 17.5C38 7 56 8 56.5 21C66 20 70 30 64 36C67 41 62 42 59 42Z"
        fill="#fbfaff"
        stroke={INK}
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      <path d="M22 33q4 3 8 0" fill="none" stroke={INK} strokeWidth="1" strokeLinecap="round" opacity="0.35" />
    </svg>
  );
}

export function Stage() {
  const live = useStore((s) => selectedVolume(s.status?.volumes));
  const hold = useMascotStage((s) => s.hold);
  const [view, setView] = useState<View | null>(null);
  const sumo = useRef<HTMLDivElement>(null);
  const parcels = useRef<(HTMLDivElement | null)[]>([]);
  const prop = useRef<HTMLDivElement>(null);
  const flaps = useRef<(SVGGElement | null)[]>([]);
  const flash = useRef<HTMLDivElement>(null);
  const [director] = useState(() => new Director(setView, { sumo, parcels, prop, flaps, flash }));

  useEffect(() => director.attach(), [director]);
  useLayoutEffect(() => {
    if (view) director.run(view);
  }, [director, view?.seq, view?.phase]);

  if (!view) return null;
  const L = view.layout;
  const out = view.phase === "gone";
  const loop = view.phase === "work" ? (view.move === "push" ? " m-jitter" : view.move === "flop" ? " m-squish" : "") : "";
  return (
    <div className="mascot-stage" aria-hidden>
      <div className="m-at" style={{ left: L.ladder.x, top: L.ladder.top }}>
        <Ladder height={L.ladder.height} sag={view.sag} out={out} />
      </div>
      {view.prop && (
        <div className="m-at" style={view.prop === "box" ? { left: L.box.x, top: L.box.y } : { left: L.cloud.x, top: L.cloud.y }}>
          <div ref={prop} className="m-pop">
            {view.prop === "box" ? <BoxArt flaps={flaps} /> : <CloudArt />}
          </div>
        </div>
      )}
      {!out && (
        <div className="m-at" style={{ left: L.sx, top: L.sy, width: L.S, height: L.S, zIndex: view.move === "push" || view.move === "flop" ? 1 : 0 }}>
          <div ref={sumo} className="h-full w-full">
            <SumoSvg act={view.act} volume={hold ?? live} className="h-full w-full" />
          </div>
        </div>
      )}
      {!out &&
        L.parcels.map((p, i) => (
          <div key={i} className="m-at" style={{ left: p.x, top: p.y }}>
            <div ref={(el) => void (parcels.current[i] = el)}>
              <div className={`m-parcel${loop}`}>
                <ParcelArt p={view.parcel} />
              </div>
            </div>
          </div>
        ))}
      {view.move === "clap" && !out && (
        <div className="m-at" style={{ left: L.sx + L.S / 2 - 15, top: L.ground - PH / 2 - 15 }}>
          <div ref={flash} className="m-flash" />
        </div>
      )}
    </div>
  );
}
