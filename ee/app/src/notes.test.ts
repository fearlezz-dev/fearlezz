import { describe, expect, test } from "bun:test";
import { applyWork } from "./live";
import { type Draft, elsewhere, fromWhere, linkedWith, notesOn, onSide, regionBetween, sidesOf } from "./notes";
import { parts } from "./pictures";
import type { Shown as Picture } from "./pictures";
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
    const resolved = applyWork(added, 3, { kind: "resolved", notes: [note("a", { state: "resolved", resolved_by: "writer" })] });
    expect(resolved.notes?.map((n) => n.id)).toEqual(["b"]);
    expect(applyWork(resolved, 3, { kind: "note", note: note("c") })).toBe(resolved);
  });

  test("notes resolved together, as a linked group is, all leave", () => {
    const s = state([note("a"), note("b", { link: "a" }), note("c")]);
    const gone = applyWork(s, 2, { kind: "resolved", notes: [note("a", { state: "resolved" }), note("b", { link: "a", state: "resolved" })] });
    expect(gone.notes?.map((n) => n.id)).toEqual(["c"]);
  });
});

describe("elsewhere", () => {
  const size = (w: number, h: number) => ({ size_before: [w, h], size_after: [w, h] }) as never;
  const shown = (name: string, more: { before?: boolean; size?: [number, number] } = {}): Picture =>
    ({
      key: `screens/${name}`,
      check: "screens",
      look: {} as never,
      parts: parts(name),
      picture: {
        name,
        kind: "still",
        change: "changed",
        summary: "",
        before: more.before === false ? null : `b/${name}`,
        after: `a/${name}`,
        comparison: more.size ? size(...more.size) : null,
      },
    }) as Picture;
  const light = shown("card/desktop/light.png", { size: [1440, 900] });
  const dark = shown("card/desktop/dark.png", { size: [1440, 900] });
  const phone = shown("card/mobile/light.png", { size: [412, 915], before: false });

  test("the open notes on a screen's other variants, not those already here in the same words", () => {
    const s = state([
      note("a", { picture: light.picture.name, words: "banner overlaps" }),
      note("b", { picture: light.picture.name, words: "already here" }),
      note("c", { picture: dark.picture.name, words: "already here" }),
      note("d", { picture: light.picture.name, words: "resolved", state: "resolved" }),
    ]);
    const from = elsewhere(s, dark, [light, dark, phone]);
    expect(from.map((f) => [f.note.id, f.where, f.place])).toEqual([["a", "desktop / light", true]]);
  });

  test("the same words on several variants are one, naming each, copied from one of the same size", () => {
    const tablet = shown("card/tablet/light.png", { size: [800, 1000] });
    const s = state([
      note("p", { picture: phone.picture.name, words: "too tight" }),
      note("a", { picture: light.picture.name, words: "too tight" }),
      note("t", { picture: tablet.picture.name, words: "too tight" }),
    ]);
    const from = elsewhere(s, dark, [light, dark, phone, tablet]);
    expect(from.map((f) => [f.note.id, f.where, f.place])).toEqual([["a", "desktop / light, mobile / light, tablet / light", true]]);
  });

  test("a picture of another size takes the words, not the place", () => {
    const s = state([note("a", { picture: light.picture.name, side: "both" })]);
    const [from] = elsewhere(s, phone, [light, dark, phone]);
    expect(from!.place).toBe(false);
    // A new picture has only its after.
    expect(from!.side).toBe("after");
  });
});

describe("linkedWith", () => {
  test("the notes made from the same first note, open, but the note itself", () => {
    const first = note("n1");
    const copy = note("n2", { picture: "card-dark.png", link: "n1" });
    const other = note("n3", { link: "n1", state: "resolved" });
    const alone = note("n4");
    const state = { seq: 1, work: { repository: "o/r", branch: "b" }, runs: [], notes: [first, copy, other, alone] } as Snapshot;
    expect(linkedWith(state, first).map((n) => n.id)).toEqual(["n2"]);
    expect(linkedWith(state, copy).map((n) => n.id)).toEqual(["n1"]);
    expect(linkedWith(state, alone)).toEqual([]);
  });
});
