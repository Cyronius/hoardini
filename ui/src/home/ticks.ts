import { routes } from "../api/client";
import type { Proposal } from "../api/types";
import { heldTicks, holdTicks, useStore } from "../store";
import { picked, type Part } from "./groups";

const sending = new Set<string>();

// One PUT in flight per proposal; ticks made meanwhile go in the next one, so replies never land out of order.
async function flush(id: string) {
  if (sending.has(id)) return;
  sending.add(id);
  try {
    for (let want = heldTicks(id); want; want = heldTicks(id)) {
      const p = await routes.selectItems(id, [...want]);
      if (heldTicks(id) !== want) continue;
      holdTicks(id, null);
      useStore.getState().putProposal(p);
    }
  } catch {
    holdTicks(id, null);
    void useStore.getState().loadProposal(id);
  } finally {
    sending.delete(id);
  }
}

export function setTicks(p: Proposal, ids: Set<number>) {
  holdTicks(p.id, ids);
  useStore.getState().putProposal({ ...p, touched: true });
  void flush(p.id);
}

export function toggleTick(p: Proposal, itemId: number) {
  const next = new Set(p.items.filter((i) => i.selected).map((i) => i.id));
  if (next.has(itemId)) next.delete(itemId);
  else next.add(itemId);
  setTicks(p, next);
}

export type Started = { jobs: string[]; errors: string[] };

// Approves every part with something ticked; each part is its own job.
export async function approveParts(parts: Part[]): Promise<Started> {
  const st = useStore.getState();
  const out: Started = { jobs: [], errors: [] };
  for (const x of parts) {
    const p = st.proposals[x.proposal.id] ?? x.proposal;
    const ids = picked(p).map((i) => i.id);
    if (ids.length === 0) continue;
    st.setApproving(p.id);
    try {
      const r = await routes.approve(p.id, ids);
      st.putProposal({ ...p, status: "approved", job_id: r.job_id });
      out.jobs.push(r.job_id);
    } catch (e) {
      out.errors.push(e instanceof Error ? e.message : String(e));
      void st.loadProposal(p.id);
    } finally {
      st.setApproving(null);
    }
  }
  return out;
}
