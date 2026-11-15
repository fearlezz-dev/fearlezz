import { describe, expect, test } from "bun:test";
import { picturesOf } from "./pictures";
import type { Picture, Review, Run, Snapshot } from "./proof";
import { carried, failedFor, localBy, standing, standingOf, verdictOf, waitsFor } from "./review";

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
    { check: "screens", picture: "card.png", key: "k1", by: "writer", at: 1, run: "r1", head: "aaa1111" },
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
      { check: "screens", name: "a.png", key: "1", standing: "waits" },
      { check: "screens", name: "b.png", key: "2", standing: "waits" },
    ],
    weakened: [{ check: "tests", how: "it is gone" }],
  };

  test("it says what this person can settle; a weakening fails the proof instead of waiting", () => {
    expect(waitsFor(review, { login: "w", role: "write" })).toBe("2 pictures wait for your look at aaa1111");
    expect(waitsFor(review, { login: null })).toBe("2 pictures wait for a look at aaa1111");
  });
});

describe("failedFor", () => {
  const review: Review = { head: "aaa1111bbb", state: "failed", pictures: [], weakened: [{ check: "tests", how: "it is gone" }] };

  test("a proof that fails on what it weakens says it waits for an admin's accept", () => {
    expect(failedFor(review, { login: "w", role: "write" })).toBe(
      "The proof of aaa1111 failed: what it weakens waits for an admin's accept",
    );
    expect(failedFor(review, { login: "a", role: "admin" })).toBe(
      "The proof of aaa1111 failed: what it weakens waits for your accept",
    );
    const accept = { head: "aaa1111bbb", weakened: review.weakened, by: "admin", at: 1 };
    expect(failedFor({ ...review, accept }, undefined)).toBe("The proof of aaa1111 failed");
    expect(failedFor({ ...review, weakened: [] }, undefined)).toBe("The proof of aaa1111 failed");
  });
});

const note = (id: string, picture: string, state: "open" | "resolved" = "open") => ({
  id,
  check: "screens",
  picture,
  run: "r1",
  head: "aaa1111",
  side: "after" as const,
  region: { x: 0, y: 0, w: 1, h: 1 },
  words: "too tight",
  by: "writer",
  at: 2,
  state,
});

describe("needs change", () => {
  test("the headline counts the pictures open notes are on", () => {
    const review: Review = {
      head: "aaa1111bbb",
      state: "needs_change",
      pictures: [
        { check: "screens", name: "a.png", key: "1", standing: "needs_change" },
        { check: "screens", name: "b.png", key: "2", standing: "waits" },
      ],
      weakened: [],
      notes: [note("n1", "a.png"), note("n2", "a.png")],
    };
    expect(standing(review)).toBe("1 picture needs change at aaa1111");
  });

  test("an open note on a picture outweighs looks right, until it is resolved", () => {
    const first = run("r1", "aaa1111", "k1");
    const shown = picturesOf(first)[0]!;
    expect(standingOf(state([first]), first, shown)).toBe("looks_right");
    const noted = { ...state([first]), notes: [note("n1", "card.png")] };
    expect(standingOf(noted, first, shown)).toBe("needs_change");
    const resolved = { ...state([first]), notes: [note("n1", "card.png", "resolved")] };
    expect(standingOf(resolved, first, shown)).toBe("looks_right");
    expect(standingOf({ ...resolved, verdicts: [] }, first, shown)).toBe("none");
  });
});

describe("a local run", () => {
  test("says who ran it, and a CI run is no one's", () => {
    const local: Run = { ...run("l1", "abc", "k"), source: { kind: "local", login: "writer" } };
    expect(localBy(local)).toBe("writer");
    expect(localBy({ ...local, source: { kind: "local", login: "" } })).toBe("a developer");
    expect(localBy(run("c1", "abc", "k"))).toBeUndefined();
  });
});
