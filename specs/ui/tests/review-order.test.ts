// Traces: UI-REVIEW-ORDER
import { describe, expect, it } from "vitest";
import type { Proposal, ProposalItem, ReviewCardRef } from "@ui/api/types";
import { chainOf, orderCards, readyBytes } from "@ui/review/order";

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

function prop(id: string, status: Proposal["status"], items: ProposalItem[], follow_up_id: string | null = null): Proposal {
  return {
    id,
    kind: "delete",
    status,
    total_alloc: items.reduce((a, i) => a + i.alloc, 0),
    created_ms: 0,
    job_id: null,
    destination: null,
    format: null,
    summary: "",
    items,
    parent_id: null,
    follow_up_id,
    review_id: "r1",
  };
}

const ref = (id: string): ReviewCardRef => ({ card: id, title: id, proposal_id: id });

describe("UI-REVIEW-ORDER", () => {
  it("sums only the ticked, pending items of pending cards", () => {
    const pending = prop("a", "pending", [
      item(10, true),
      item(20, false),
      item(30, true, { status: "done" }),
      item(40, true, { block_reason: "open in another program", status: "blocked" }),
    ]);
    const declined = prop("b", "declined", [item(50, true)]);
    expect(readyBytes([pending, declined, undefined])).toBe(10 * GB);
  });

  it("counts a compress item by what its estimate saves, not its size", () => {
    const compress = { ...prop("c", "pending", [item(7.5, true, { saves: Math.round(3.2 * GB) }), item(2, true)]), kind: "compress" as const };
    expect(readyBytes([compress])).toBe(Math.round(3.2 * GB) + 2 * GB);
  });

  it("follows a card into its follow-ups and stops on a loop", () => {
    const ps = { a: prop("a", "done", [], "b"), b: prop("b", "pending", [], "c"), c: prop("c", "pending", []) };
    expect(chainOf("a", ps).map((p) => p.id)).toEqual(["a", "b", "c"]);
    const loop = { a: prop("a", "done", [], "b"), b: prop("b", "done", [], "a") };
    expect(chainOf("a", loop).map((p) => p.id)).toEqual(["a", "b"]);
    expect(chainOf("gone", ps)).toEqual([]);
  });

  it("puts the most ready to free first, then unticked, running, finished, replaced and unknown cards", () => {
    const ps: Record<string, Proposal> = {
      superseded: prop("superseded", "superseded", [item(200, true)]),
      downloads: prop("downloads", "pending", [item(7, false), item(5.9, false), item(2.5, false)]),
      temp: prop("temp", "pending", [item(33, true), item(23, true), item(17.8, true), item(17.6, true), item(3.1, true)]),
      declined: prop("declined", "declined", [item(80, true)]),
      build: prop("build", "pending", [item(61, true), item(52, true), item(6.2, false)]),
      running: prop("running", "running", [item(90, true)]),
      archive: prop("archive", "pending", [item(11.2, false)]),
      done: prop("done", "done", [item(70, true, { status: "done" })], "follow"),
      follow: prop("follow", "pending", [item(5, true)]),
    };
    const cards = ["superseded", "downloads", "temp", "declined", "missing", "build", "running", "archive", "done"].map(ref);
    expect(orderCards(cards, ps).map((c) => c.card)).toEqual(["build", "temp", "done", "downloads", "archive", "running", "declined", "superseded", "missing"]);
  });

  it("keeps the input order between equal cards", () => {
    const ps = { x: prop("x", "pending", [item(4, true)]), y: prop("y", "pending", [item(4, true)]) };
    expect(orderCards([ref("y"), ref("x")], ps).map((c) => c.card)).toEqual(["y", "x"]);
  });
});
