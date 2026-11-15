import { describe, expect, test } from "bun:test";
import { applyWork } from "./live";
import { type Draft, fromWhere, notesOn, onSide, regionBetween, sidesOf } from "./notes";
import type { Note, Run, Snapshot } from "./proof";

const note = (id: string, fields: Partial<Note> = {}): Note => ({
  id,
  check: "screens",
  picture: "card.png",
  run: "r1",
  head: "aaa1111bbb",
  side: "after",
  region: { x: 0.1, y: 0.6, w: 0.3, h: 0.2 },
  words: `note ${id}`,
  by: "writer",
  at: 10,
  state: "open",
  ...fields,
});
const run = (id: string, head: string): Run => ({
  id,
  head,
  base: "main",
  source: { kind: "ci", job: "proof", run_id: 1, run_attempt: 1, runner_name: "runner" },
  state: "done",
  opened: 1,
  checks: [],
});
const state = (notes: Note[]): Snapshot => ({
  seq: 1,
  work: { repository: "owner/repo", branch: "feat/x" },
  runs: [],
  notes,
});

describe("notesOn", () => {
  test("the open notes on one picture, numbered in the order made, old ones marked", () => {
    const s = state([
      note("b", { at: 20, run: "r2", head: "bbb2222" }),
      note("a", { at: 10 }),
      note("other", { picture: "menu.png" }),
      note("done", { state: "resolved" }),
    ]);
    const shown = notesOn(s, "screens", "card.png", run("r2", "bbb2222"));
    expect(shown.map((n) => [n.note.id, n.n, n.old])).toEqual([
      ["a", 1, true],
      ["b", 2, false],
    ]);
  });
});

describe("onSide", () => {
  test("a note shows on its side, and on both for both", () => {
    expect(onSide(note("a"), "after")).toBe(true);
    expect(onSide(note("a"), "before")).toBe(false);
    expect(onSide(note("a", { side: "both" }), "before")).toBe(true);
  });
});

describe("fromWhere", () => {
  test("an old note says the run it was made on", () => {
    expect(fromWhere(note("a"), run("r1", "aaa1111bbb"))).toBe("");
    expect(fromWhere(note("a"), run("r2", "aaa1111bbb"))).toBe("from an earlier run");
    expect(fromWhere(note("a"), run("r2", "bbb2222ccc"))).toBe("from an earlier push (aaa1111)");
  });
});

describe("regionBetween", () => {
  test("two points in any order make a region, kept on the picture", () => {
    expect(regionBetween([0.4, 0.8], [0.1, 0.6])).toEqual({ x: 0.1, y: 0.6, w: 0.30000000000000004, h: 0.20000000000000007 });
    const off = regionBetween([-0.2, 0.5], [1.4, 0.7]);
    expect(off.x).toBe(0);
    expect(off.w).toBe(1);
  });
});

describe("sidesOf", () => {
  test("a draft on both sides needs both, and falls back to the side drawn on", () => {
    const draft: Draft = { side: "before", region: { x: 0, y: 0, w: 1, h: 1 } };
    expect(sidesOf(draft, false)).toBe("before");
    expect(sidesOf(draft, true)).toBe("both");
  });
});

describe("applyWork with notes", () => {
  test("a new note joins, a resolved one leaves, an event held already changes nothing", () => {
    const s = state([note("a")]);
    const added = applyWork(s, 2, { kind: "note", note: note("b", { at: 20 }) });
    expect(added.notes?.map((n) => n.id)).toEqual(["a", "b"]);
    const resolved = applyWork(added, 3, { kind: "note", note: note("a", { state: "resolved", resolved_by: "writer" }) });
    expect(resolved.notes?.map((n) => n.id)).toEqual(["b"]);
    expect(applyWork(resolved, 3, { kind: "note", note: note("c") })).toBe(resolved);
  });
});
