import type { Proposal } from "../api/types";

interface Deferred {
  promise: Promise<string>;
  resolve: (v: string) => void;
}

// One entry per proposal id, so parallel cards never overwrite each other.
const entries = new Map<string, Deferred>();

export function decisionFor(proposalId: string): Deferred {
  let d = entries.get(proposalId);
  if (!d) {
    let resolve!: (v: string) => void;
    const promise = new Promise<string>((r) => (resolve = r));
    d = { promise, resolve };
    entries.set(proposalId, d);
  }
  return d;
}

export function settle(proposalId: string, result: unknown): void {
  decisionFor(proposalId).resolve(JSON.stringify(result));
}

export function outcomeOf(p: Proposal): { approved: boolean; items?: number[]; job_id?: string | null; status: string } {
  if (p.status === "declined") return { approved: false, status: p.status };
  return {
    approved: true,
    items: p.items.filter((i) => i.selected).map((i) => i.id),
    job_id: p.job_id,
    status: p.status,
  };
}
