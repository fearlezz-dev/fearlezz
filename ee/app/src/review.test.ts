import { describe, expect, test } from "bun:test";
import { picturesOf } from "./pictures";
import type { Picture, Review, Run, Snapshot } from "./proof";
import { carried, verdictOf, waitsFor } from "./review";

const picture: Picture = {
  name: "card.png",
  kind: "still",
  change: "changed",
  summary: "",
  before: "looks/s/before/card.png",
  after: "looks/s/after/card.png",
  comparison: null,
};
const run = (id: string, head: string, key: string): Run => ({
  id,
  head,
  base: "main",
  source: { kind: "ci", job: "proof" },
  state: "done",
  opened: 1,
  checks: [
    { id: "screens", look: { check: "screens", base_commit: "b", base_result: "passed", items: [picture] }, keys: { "card.png": key } },
  ],
});
const state = (runs: Run[]): Snapshot => ({
  seq: 1,
  work: { repository: "owner/repo", branch: "feat/x" },
  runs,
  verdicts: [
    { check: "screens", picture: "card.png", key: "k1", kind: "looks_right", by: "writer", at: 1, run: "r1", head: "aaa1111" },
  ],
});

describe("verdictOf", () => {
  test("a verdict holds for the same content on any run, and says where it came from", () => {
    const first = run("r1", "aaa1111", "k1");
    const later = run("r2", "bbb2222", "k1");
    const other = run("r3", "ccc3333", "k2");
    const s = state([other, later, first]);
    const shown = (r: Run) => picturesOf(r)[0]!;
    expect(verdictOf(s, first, shown(first))?.by).toBe("writer");
    expect(carried(verdictOf(s, first, shown(first))!, first)).toBe("");
    expect(carried(verdictOf(s, later, shown(later))!, later)).toBe("carried over from an earlier push (aaa1111)");
    expect(verdictOf(s, other, shown(other))).toBeUndefined();
  });
});

describe("waitsFor", () => {
  const review: Review = {
    head: "aaa1111bbb",
    state: "waits",
    pictures: [
      { check: "screens", name: "a.png", key: "1" },
      { check: "screens", name: "b.png", key: "2" },
    ],
    weakened: [{ check: "tests", how: "it is gone" }],
  };

  test("it says what this person can settle, and what waits for others", () => {
    expect(waitsFor(review, { login: "w", role: "write" })).toBe(
      "2 pictures wait for your look, and a weakened proof waits for an admin at aaa1111",
    );
    expect(waitsFor(review, { login: "a", role: "admin" })).toBe(
      "2 pictures wait for your look, and a weakened proof waits for your accept at aaa1111",
    );
    expect(waitsFor({ ...review, weakened: [] }, { login: null })).toBe("2 pictures wait for a look at aaa1111");
  });
});
