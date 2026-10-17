// Traces: UI-HOME
import { describe, expect, it } from "vitest";
import type { Proposal, ProposalItem, RarelyUsedApp, ReviewArea, ReviewView } from "@ui/api/types";
import { homeOf } from "@ui/home/groups";

const GB = 1024 ** 3;
let seq = 0;

function item(gb: number, selected: boolean, more: Partial<ProposalItem> = {}): ProposalItem {
  return {
    id: ++seq,
    path: `C:\\x\\${seq}`,
    kind: "dir",
    alloc: Math.round(gb * GB),
    verdict: "safe",
    cap: null,
    reason: null,
    block_reason: null,
    selected,
    status: "pending",
    detail: null,
    small: false,
    memory_note: null,
    ...more,
  };
}

function prop(id: string, items: ProposalItem[], status: Proposal["status"] = "pending"): Proposal {
  return { id, kind: "delete", status, total_alloc: 0, created_ms: 0, job_id: null, destination: null, format: null, summary: id, items, parent_id: null, follow_up_id: null, review_id: "r1" };
}

function area(path: string, gb: number, more: Partial<ReviewArea> = {}): ReviewArea {
  return { path, size: gb * GB, label: "unknown", category: "unknown", verdict: null, why_not: "", hint: null, unknown: true, actions: [], ...more };
}

function view(props: Proposal[], areas: ReviewArea[] = [], apps: RarelyUsedApp[] = []): ReviewView {
  return {
    review: { id: "r1", created_ms: 1, volume: "C:\\" },
    header: { root: "C:\\", used_bytes: 0, free_bytes: 0, ready_bytes: 0 },
    cards: props.map((p) => ({ card: p.id, title: p.id, proposal_id: p.id })),
    areas,
    proposals: Object.fromEntries(props.map((p) => [p.id, p])),
    apps,
  };
}

const byId = (ps: Proposal[]) => Object.fromEntries(ps.map((p) => [p.id, p]));

describe("UI-HOME groups", () => {
  it("puts junk and keep-the-files cards in their groups and counts only ticked, unblocked, pending items", () => {
    const ps = [
      prop("temp", [item(2, true), item(1, false), item(4, true, { block_reason: "In use" })]),
      prop("caches", [item(3, true)]),
      prop("windows", [item(0, true, { op: { op: "component_store", label: "", explain: "", elevated: true, irreversible: false, estimate_known: false, size: null } })]),
      prop("compress", [item(10, true, { saves: 4 * GB })]),
      prop("online_only", [item(1, true)]),
    ];
    const h = homeOf(view(ps), byId(ps));
    expect(h.junk.parts.map((p) => p.card)).toEqual(["temp", "caches", "windows"]);
    expect(h.junk.bytes).toBe(5 * GB);
    expect(h.junk.count).toBe(3);
    expect(h.junk.atLeast).toBe(true);
    expect(h.keep.parts.map((p) => p.card)).toEqual(["compress", "online_only"]);
    expect(h.keep.bytes).toBe(5 * GB);
    expect(h.freeable).toBe(10 * GB);
    expect(h.calls).toEqual([]);
  });

  it("leaves out cards that ran or have nothing left to choose", () => {
    const ps = [prop("temp", [item(2, true)], "done"), prop("caches", [item(3, true, { status: "done" })]), prop("build", [item(1, false)])];
    const h = homeOf(view(ps), byId(ps));
    expect(h.junk.parts.map((p) => p.card)).toEqual(["build"]);
    expect(h.junk.bytes).toBe(0);
  });

  it("lists areas, apps and the judgment cards as Your call rows, biggest first; user keeps go to kept; hands-off areas (system, or a keep with no action left) are hidden", () => {
    const ps = [prop("downloads", [item(2, false), item(1, false)])];
    const areas = [
      area("C:\\Users\\a\\Desktop\\EncryptedClips", 51, { unknown: false, label: "your photos and videos", category: "photos", verdict: "keep", actions: ["archive"] }),
      area("C:\\ProgramData", 40, { unknown: false, label: "app data for every user", category: "app-data", verdict: "keep" }),
      area("C:\\Users\\a\\Kept", 20, { unknown: false, label: "yours", category: "user", verdict: "keep", user: true }),
      area("C:\\Windows", 30, { category: "system", unknown: false, verdict: "keep" }),
      area("C:\\Big", 9),
    ];
    const apps: RarelyUsedApp[] = [{ id: "steam-1", name: "Elden Ring", publisher: null, source: "steam", path: "C:\\Games\\Elden Ring", size: 48 * GB, last_run_ms: null, last_run_from: null }];
    const h = homeOf(view(ps, areas, apps), byId(ps));
    expect(h.calls.map((c) => c.name)).toEqual(["EncryptedClips", "Elden Ring", "Big", "Installers and archives in Downloads"]);
    expect(h.calls[0]!.desc).toBe("Your photos and videos");
    expect(h.calls[1]!.desc).toBe("Steam game, never played");
    expect(h.calls[2]!.desc).toBe("Hoardini doesn't know what this is yet");
    expect(h.calls[3]!.bytes).toBe(3 * GB);
    expect(h.kept.map((a) => a.path)).toEqual(["C:\\Users\\a\\Kept"]);
    expect(h.junk.parts).toEqual([]);
  });

  it("drops rows for paths acted on this session, and anything inside them", () => {
    const areas = [area("C:\\Big", 9), area("C:\\Other", 5)];
    const h = homeOf(view([], areas), {}, new Set(["c:\\big"]));
    expect(h.calls.map((c) => c.name)).toEqual(["Other"]);
  });

  it("prefers the live proposal from the store over the view's copy", () => {
    const stale = prop("temp", [item(2, true)]);
    const live = prop("temp", [{ ...stale.items[0]!, selected: false }]);
    const h = homeOf(view([stale]), { temp: live });
    expect(h.junk.bytes).toBe(0);
  });
});
