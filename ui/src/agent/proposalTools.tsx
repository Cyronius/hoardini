import type { ToolDefinition } from "@itkennel/lm-ag-ui";
import { ProposalCard } from "../components/ProposalCard";
import { useStore } from "../store";
import { decisionFor, outcomeOf } from "./decisions";
import { PROPOSE_TOOLS, proposeToolDefs } from "./toolDefs";

async function handler(args: { proposal_id?: string } | null): Promise<string> {
  const id = args?.proposal_id;
  if (!id) return JSON.stringify({ ok: false, error: "missing_proposal_id" });
  const p = await useStore.getState().loadProposal(id);
  if (!p) return JSON.stringify({ ok: false, error: "proposal_not_found" });
  if (p.status !== "pending") return JSON.stringify(outcomeOf(p));
  return decisionFor(id).promise;
}

export function proposalTools(): Record<string, ToolDefinition> {
  const out: Record<string, ToolDefinition> = {};
  for (const name of PROPOSE_TOOLS) {
    out[name] = {
      definition: proposeToolDefs[name],
      isFrontend: true,
      handler: (args) => handler(args),
      renderer: (args) => {
        const id = (args as { proposal_id?: string } | null)?.proposal_id;
        return id ? <ProposalCard key={id} proposalId={id} /> : <div className="text-muted pulse-soft">Preparing proposal...</div>;
      },
    };
  }
  return out;
}
