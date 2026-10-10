import type { CardData, PillKind, ToolData } from "../api/types";
import { useExploreLink } from "../explore/context";
import { fileName, humanBytes, isUnder, samePath } from "../format";
import { starter } from "./Greeting";
import { askAbout, cleanUpIn, isCardData } from "./ToolCards";

export const MAX_FOLLOW_UPS = 3;

export type FollowUp = { label: string; kind: PillKind; path?: string; send: string };

function fromCard(d: CardData): FollowUp[] {
  if (d.kind === "folders") {
    const top = d.items[0];
    const scope = d.under ?? top?.path;
    const out: FollowUp[] = [];
    if (top) out.push({ label: `What's inside ${fileName(top.path)} (${humanBytes(top.size)})?`, kind: "ask", path: top.path, send: `What's inside ${top.path}?` });
    if (scope) out.push({ label: `Clean up ${scope.length > 24 ? fileName(scope) : scope}`, kind: "clean_up", path: scope, send: cleanUpIn(scope) });
    return out;
  }
  if (d.kind === "files") {
    const top = d.items[0];
    return top ? [{ label: `What is ${fileName(top.path)} (${humanBytes(top.size)})?`, kind: "ask", path: top.path, send: askAbout(top.path) }] : [];
  }
  const grew = [...d.grew, ...d.new].filter((c) => !samePath(c.path, d.volume));
  const top = grew
    .filter((c) => !grew.some((o) => o !== c && isUnder(o.path, c.path)))
    .sort((a, b) => b.delta - a.delta)[0];
  return top ? [{ label: `Why did ${fileName(top.path)} grow (+${humanBytes(top.delta)})?`, kind: "ask", path: top.path, send: `What is ${top.path} and why did it grow?` }] : [];
}

// The model's own suggestions lead; pills built from the turn's cards only fill what is left.
export function followUps(turn: ToolData[], can: { explore?: boolean } = {}): FollowUp[] {
  const out: FollowUp[] = [];
  const seen = new Set<string>();
  const add = (p: FollowUp) => {
    const key = `${p.kind}|${p.path ? p.path.toLowerCase() : `label:${p.label.toLowerCase()}`}`;
    if (out.length >= MAX_FOLLOW_UPS || seen.has(key)) return;
    seen.add(key);
    out.push(p);
  };
  for (const d of turn) {
    if (d.kind !== "pills") continue;
    for (const p of d.items) {
      if (p.kind === "explore" && !(can.explore && p.path)) continue;
      add({ ...p, send: p.kind === "clean_up" && p.path ? cleanUpIn(p.path) : p.kind === "explore" ? `Open ${p.path} in Explore` : p.label });
    }
  }
  for (const d of turn) if (isCardData(d)) fromCard(d).forEach(add);
  return out;
}

export function FollowUps({ items, onAsk, disabled }: { items: FollowUp[]; onAsk: (text: string) => void; disabled?: boolean }) {
  const { onOpenExplore } = useExploreLink();
  if (items.length === 0) return null;
  return (
    <div className="fade-up flex flex-wrap gap-2" role="group" aria-label="Suggested next steps">
      {items.map((p) => (
        <button key={`${p.kind}|${p.path ?? p.label}`} type="button" className={`${starter} disabled:opacity-40`} title={p.send} disabled={disabled} onClick={() => (p.kind === "explore" && p.path ? onOpenExplore?.(p.path) : onAsk(p.send))}>
          {p.label}
        </button>
      ))}
    </div>
  );
}
