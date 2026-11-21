import { describe, expect, test } from "bun:test";
import type { Said } from "./api";
import { hoped, hoping, seen, verdictChange } from "./hopes";
import type { Note, Snapshot } from "./proof";

const note = (id: string, fields: Partial<Note> = {}): Note => ({
  id,
  check: "screens",
  picture: "card.png",
  run: "r1",
  head: "aaa1111",
  side: "after",
  region: { x: 0.1, y: 0.6, w: 0.3, h: 0.2 },
  words: `note ${id}`,
  by: "writer",
  at: 10,
  state: "open",
  ...fields,
});
const state: Snapshot = { seq: 4, work: { repository: "owner/repo", branch: "feat/x" }, runs: [], notes: [note("a")] };
const verdict = { check: "screens", picture: "card.png", key: "k1", by: "writer", at: 20, run: "r1", head: "aaa1111" };

/** An answer the test gives when it likes, as the host far away would. */
function later<T>() {
  let give: (said: Said<T>) => void = () => {};
  const asked = new Promise<Said<T>>((resolve) => (give = resolve));
  return { ask: () => asked, give };
}

describe("hoped", () => {
  test("what a person said shows at once, as the host's event would, at the host's sequence", () => {
    const shown = hoped(state, [verdictChange(verdict, [note("a")])]);
    expect(shown.verdicts).toEqual([verdict]);
    expect(shown.notes).toEqual([]);
    expect(shown.seq).toBe(4);
  });

  test("with nothing said, the work is the host's own", () => {
    expect(hoped(state, [])).toBe(state);
  });
});

describe("hoping", () => {
  test("a refusal takes back what showed, and says why", async () => {
    const host = later();
    const said = hoping({ kind: "note", note: note("pending-1") }, host.ask);
    expect(hoped(state).notes?.map((n) => n.id)).toEqual(["a", "pending-1"]);
    host.give({ ok: false, error: "no write access" });
    expect(await said).toBe("no write access");
    expect(hoped(state).notes?.map((n) => n.id)).toEqual(["a"]);
  });

  test("taken, it shows as the host named it until the host's event comes", async () => {
    const host = later<{ note: Note }>();
    const said = hoping({ kind: "note", note: note("pending-2") }, host.ask, (answer) => ({ kind: "note", note: answer.note }));
    host.give({ ok: true, answer: { note: note("n7") } });
    expect(await said).toBeUndefined();
    expect(hoped(state).notes?.map((n) => n.id)).toEqual(["a", "n7"]);
    // The live connection brings the work with it: the hope is done.
    seen();
    expect(hoped(state).notes?.map((n) => n.id)).toEqual(["a"]);
  });

  test("a note the host's event brought before it answered shows once", async () => {
    const host = later<{ note: Note }>();
    const mine = note("pending-9", { words: "Too tight" });
    const said = hoping({ kind: "note", note: mine }, host.ask, (answer) => ({ kind: "note", note: answer.note }));
    const arrived: Snapshot = { ...state, notes: [note("a"), { ...mine, id: "n9" }] };
    expect(hoped(arrived).notes?.map((n) => n.id)).toEqual(["a", "n9"]);
    host.give({ ok: true, answer: { note: { ...mine, id: "n9" } } });
    await said;
    expect(hoped(arrived).notes?.map((n) => n.id)).toEqual(["a", "n9"]);
    seen();
  });

  test("an event before the host answers leaves what is still on its way", async () => {
    const host = later();
    const said = hoping({ kind: "note", note: resolvedOf(note("a")) }, host.ask);
    seen();
    expect(hoped(state).notes).toEqual([]);
    host.give({ ok: true, answer: {} });
    await said;
    seen();
  });
});

const resolvedOf = (n: Note): Note => ({ ...n, state: "resolved", resolved_by: "writer" });
