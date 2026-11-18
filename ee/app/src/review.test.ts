import { describe, expect, test } from "bun:test";
import { picturesOf } from "./pictures";
import type { Picture, Run, Snapshot } from "./proof";
import { canAccept, canSay, carried, isViewer, localBy, roleOf, standingOf, verdictOf } from "./review";

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
  source: { kind: "ci", job: "proof", run_id: 1, run_attempt: 1, runner_name: "runner" },
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

describe("who looks", () => {
  test("the host's line for each role is picked by what the person may do", () => {
    const title = { read: "1 picture waits for a look at aaa1111", write: "1 picture waits for your look at aaa1111", admin: "x" };
    expect(title[roleOf({ login: "w", role: "write" })]).toBe("1 picture waits for your look at aaa1111");
    expect(title[roleOf({ login: null })]).toBe("1 picture waits for a look at aaa1111");
    expect(title[roleOf("shared")]).toBe("1 picture waits for a look at aaa1111");
    expect(roleOf(undefined)).toBe("read");
    expect(canSay({ login: "w", role: "write" })).toBe(true);
    expect(canSay({ login: "r", role: "read" })).toBe(false);
    expect(canAccept({ login: "w", role: "write" })).toBe(false);
    expect(canAccept({ login: "a", role: "admin" })).toBe(true);
  });

  test("only a viewer as the host answers it is taken", () => {
    expect(isViewer({ login: null })).toBe(true);
    expect(isViewer({ login: "w", role: "write" })).toBe(true);
    expect(isViewer({ login: "w", role: "owner" })).toBe(false);
    expect(isViewer({ login: "w" })).toBe(false);
    expect(isViewer({ error: "sign in" })).toBe(false);
    expect(isViewer(null)).toBe(false);
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
