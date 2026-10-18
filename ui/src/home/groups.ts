import type { Proposal, ProposalItem, RarelyUsedApp, ReviewArea, ReviewView } from "../api/types";
import { fileName, isUnder, samePath, shortDate } from "../format";

export type GroupId = "junk" | "keep";

// Which review cards make up each Home group; the rest of the cards are Your call rows (UI-HOME).
export const GROUP_CARDS: Record<GroupId, readonly string[]> = {
  junk: ["temp", "caches", "build", "windows", "disposable"],
  keep: ["compress", "online_only"],
};
export const CALL_CARDS: readonly string[] = ["downloads", "redownload", "archive"];

export const PART_NAME: Record<string, string> = {
  temp: "Temporary files",
  caches: "App and browser caches",
  build: "Build files",
  windows: "Windows leftovers",
  disposable: "Folders you said can go",
  compress: "Shrink in place",
  online_only: "Keep in the cloud only",
  downloads: "Installers and archives in Downloads",
  redownload: "Files you can download again",
  archive: "Old photos, videos and documents",
};

export const PART_NOTE: Record<string, string> = {
  temp: "Leftovers from Windows and your apps, untouched for a day",
  caches: "Your apps and browsers download these again as needed",
  build: "Your projects rebuild these",
  windows: "Windows asks for permission once",
  disposable: "You said these can go",
  compress: "Files stay where they are and open as before",
  online_only: "Files stay in your cloud folder and download again when opened",
  downloads: "Most of these can be downloaded again",
  redownload: "Each has an identical copy online",
  archive: "Not opened in six months; they move to your other drive",
};

export type Part = { card: string; name: string; note: string; proposal: Proposal };
export type Group = { id: GroupId; parts: Part[]; bytes: number; count: number; atLeast: boolean };

type RowBase = { key: string; name: string; desc: string; bytes: number };
export type CallRow =
  | (RowBase & { kind: "area"; path: string; area: ReviewArea })
  | (RowBase & { kind: "app"; path: string; app: RarelyUsedApp })
  | (RowBase & { kind: "part"; part: Part })
  | (RowBase & { kind: "peek"; path: string });

export type Home = { junk: Group; keep: Group; calls: CallRow[]; kept: ReviewArea[]; freeable: number };

// A compress item frees what its estimate saves, not its size (REVIEW-BUILD).
export const worth = (i: ProposalItem) => i.saves ?? i.alloc;
export const choosable = (i: ProposalItem) => i.status === "pending" && !i.block_reason;
export const picked = (p: Proposal) => p.items.filter((i) => i.selected && choosable(i));

// System, protected and container areas are hands off: never listed.
const HANDS_OFF = new Set(["system", "protected", "container"]);

const capitalized = (s: string) => (s ? s[0]!.toUpperCase() + s.slice(1) : s);

function part(card: string, p: Proposal | undefined): Part | null {
  if (!p || p.status !== "pending") return null;
  if (!p.items.some(choosable)) return null;
  return { card, name: PART_NAME[card] ?? p.summary, note: PART_NOTE[card] ?? "", proposal: p };
}

function group(id: GroupId, parts: Part[]): Group {
  const chosen = parts.flatMap((x) => picked(x.proposal));
  return {
    id,
    parts,
    bytes: chosen.reduce((a, i) => a + worth(i), 0),
    count: chosen.length,
    atLeast: chosen.some((i) => i.op && !i.op.estimate_known),
  };
}

// Same-named rows (three model.q4nx files) need the folder they live in to be told apart.
function parentTail(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean).slice(0, -1);
  return parts.length ? parts.slice(-2).join("\\") : p;
}

function areaRow(a: ReviewArea): CallRow {
  const name = fileName(a.path) || a.path;
  const desc = a.unknown ? "Hoardini doesn't know what this is yet" : capitalized(a.label);
  return { kind: "area", key: `area:${a.path.toLowerCase()}`, name, desc, bytes: a.size, path: a.path, area: a };
}

function appRow(a: RarelyUsedApp): CallRow {
  const when = a.last_run_ms ? `last opened ${shortDate(a.last_run_ms)}` : "never opened";
  const desc = a.source === "steam" ? `Steam game, ${when.replace("opened", "played")}` : `App, ${when}`;
  return { kind: "app", key: `app:${a.id}`, name: a.name, desc, bytes: a.size, path: a.path, app: a };
}

function partRow(x: Part): CallRow {
  const n = x.proposal.items.filter(choosable).length;
  const bytes = x.proposal.items.filter(choosable).reduce((s, i) => s + worth(i), 0);
  return { kind: "part", key: `part:${x.card}`, name: x.name, desc: `${n} ${n === 1 ? "item" : "items"} · ${x.note}`, bytes, part: x };
}

// `gone` holds paths acted on this session, which a stored review still lists until it is rebuilt.
export function homeOf(view: ReviewView, proposals: Record<string, Proposal | undefined>, gone: ReadonlySet<string> = new Set()): Home {
  const byCard = new Map(view.cards.map((c) => [c.card, proposals[c.proposal_id] ?? view.proposals[c.proposal_id]]));
  const isGone = (p: string) => [...gone].some((g) => samePath(g, p) || isUnder(g, p));
  // A stored proposal still lists what was acted on this session until the rebuild lands.
  const live = (p: Proposal | undefined) => (p && gone.size ? { ...p, items: p.items.filter((i) => !isGone(i.path)) } : p);
  const parts = (ids: readonly string[]) => ids.map((id) => part(id, live(byCard.get(id)))).filter((x): x is Part => !!x);
  const junk = group("junk", parts(GROUP_CARDS.junk));
  const keep = group("keep", parts(GROUP_CARDS.keep));

  const kept: ReviewArea[] = [];
  const calls: CallRow[] = [];
  for (const a of view.areas) {
    // A keep verdict with no action left is Hoardini's own hands-off call, like a system folder.
    const handsOff = HANDS_OFF.has(a.category) || (a.verdict === "keep" && a.actions.length === 0 && !a.user);
    if (handsOff || isGone(a.path)) continue;
    if (a.user && a.verdict === "keep") kept.push(a);
    else calls.push(areaRow(a));
  }
  for (const a of view.apps ?? []) if (!isGone(a.path)) calls.push(appRow(a));
  for (const x of parts(CALL_CARDS)) calls.push(partRow(x));
  calls.sort((a, b) => b.bytes - a.bytes || a.name.localeCompare(b.name));
  const seen = new Map<string, number>();
  for (const r of calls) seen.set(r.name.toLowerCase(), (seen.get(r.name.toLowerCase()) ?? 0) + 1);
  for (const r of calls) {
    if ("path" in r && r.path && (seen.get(r.name.toLowerCase()) ?? 0) > 1) r.desc = `${r.desc} · in ${parentTail(r.path)}`;
  }
  return { junk, keep, calls, kept, freeable: junk.bytes + keep.bytes };
}
