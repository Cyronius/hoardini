import type { Proposal, ReviewCardRef } from "../api/types";

type ById = Record<string, Proposal | undefined>;

// A card's follow-ups (ACT-FOLLOW-UP) carry on its work, so they count as the same card.
export function chainOf(id: string, proposals: ById): Proposal[] {
  const out: Proposal[] = [];
  const seen = new Set<string>();
  for (let p = proposals[id]; p && !seen.has(p.id); p = p.follow_up_id ? proposals[p.follow_up_id] : undefined) {
    seen.add(p.id);
    out.push(p);
  }
  return out;
}

export function readyBytes(ps: (Proposal | undefined)[]): number {
  let n = 0;
  for (const p of ps) {
    if (p?.status !== "pending") continue;
    for (const i of p.items) if (i.selected && i.status === "pending" && !i.block_reason) n += i.saves ?? i.alloc;
  }
  return n;
}

const ENDED: Proposal["status"][] = ["done", "partial", "failed", "declined"];

function rank(c: ReviewCardRef, proposals: ById): [group: number, ready: number, total: number] {
  const chain = chainOf(c.proposal_id, proposals);
  const top = chain[0];
  if (!top) return [5, 0, 0];
  if (top.status === "superseded") return [4, 0, top.total_alloc];
  const ready = readyBytes(chain);
  if (chain.some((p) => p.status === "pending")) return [ready > 0 ? 0 : 1, ready, top.total_alloc];
  if (chain.some((p) => p.status === "approved" || p.status === "running")) return [2, 0, top.total_alloc];
  return [ENDED.includes(top.status) ? 3 : 2, 0, top.total_alloc];
}

export function orderCards(cards: ReviewCardRef[], proposals: ById): ReviewCardRef[] {
  return cards
    .map((c, i) => ({ c, i, r: rank(c, proposals) }))
    .sort((a, b) => a.r[0] - b.r[0] || b.r[1] - a.r[1] || b.r[2] - a.r[2] || a.i - b.i)
    .map((x) => x.c);
}
