import { describe, expect, test } from "bun:test";
import { type Shown, picturesOf } from "./pictures";
import type { Check, Comparison, Picture, Run, Snapshot } from "./proof";
import { canAccept, canSay, carried, changedSince, covered, isViewer, localBy, opening, roleOf, standingOf, toLookAt, unstableOf, verdictOf } from "./review";

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

describe("opening", () => {
  const leg = (id: string, head: string, pictures: boolean): Run => ({ ...run(id, head, "k1"), checks: pictures ? run(id, head, "k1").checks : [{ id: "tests/macos" }] });

  test("a head proved in legs opens on the newest leg with pictures to look at", () => {
    expect(opening([leg("macos", "aaa1111", false), leg("linux", "aaa1111", true)])?.id).toBe("linux");
  });

  test("a rerun shows as it opens, even before it has pictures", () => {
    const rerun = { ...leg("again", "aaa1111", false), source: { kind: "ci", job: "proof", run_id: 1, run_attempt: 2, runner_name: "runner" } } as Run;
    expect(opening([rerun, leg("linux", "aaa1111", true)])?.id).toBe("again");
  });

  test("otherwise the newest run, and never a run of an earlier head", () => {
    expect(opening([leg("macos", "bbb2222", false), leg("linux", "aaa1111", true)])?.id).toBe("macos");
    expect(opening([])).toBeUndefined();
  });
});

describe("changed since", () => {
  const first = run("r1", "aaa1111", "k1");
  const other = run("r3", "ccc3333", "k2");
  const shown = (r: Run) => picturesOf(r)[0]!;

  test("a picture whose content changed after it looked right says who said so", () => {
    const since = changedSince(state([other, first]), other, shown(other));
    expect(since?.verdict?.by).toBe("writer");
    expect(since?.notes).toEqual([]);
  });

  test("a picture that looks right as it is has not changed since", () => {
    expect(changedSince(state([first]), first, shown(first))).toBeUndefined();
  });

  test("an open note made on other content says it changed since; one on this content does not", () => {
    const s = { ...state([other, first]), verdicts: [], notes: [note("n1", "card.png"), { ...note("n2", "card.png"), run: "r3", head: "ccc3333" }] };
    expect(changedSince(s, other, shown(other))).toEqual({ verdict: undefined, notes: ["n1"] });
  });

  test("a picture its run found the same as the one that looked right keeps the verdict", () => {
    const found = (same: boolean): Run => ({ ...other, checks: [{ ...other.checks[0]!, since: { "card.png": { k1: same } } }] });
    const same = found(true);
    expect(verdictOf(state([same, first]), same, shown(same))?.run).toBe("r1");
    expect(changedSince(state([same, first]), same, shown(same))).toBeUndefined();
    const changed = found(false);
    expect(verdictOf(state([changed, first]), changed, shown(changed))).toBeUndefined();
    expect(changedSince(state([changed, first]), changed, shown(changed))?.verdict?.run).toBe("r1");
  });

  test("an open note on a picture its run found the same is no change since", () => {
    const same: Run = { ...other, checks: [{ ...other.checks[0]!, since: { "card.png": { k1: true } } }] };
    const s = { ...state([same, first]), verdicts: [], notes: [note("n1", "card.png")] };
    expect(changedSince(s, same, shown(same))).toBeUndefined();
  });

  test("a note whose run the page does not hold says nothing", () => {
    const s = { ...state([other]), verdicts: [], notes: [note("n1", "card.png")] };
    expect(changedSince(s, other, shown(other))).toBeUndefined();
  });
});

describe("covered", () => {
  const variant = (name: string, fields: Partial<Picture> = {}): Picture => ({
    ...picture,
    name,
    kind: name.endsWith(".webm") ? "film" : "still",
    before: `looks/s/before/${name}`,
    after: `looks/s/after/${name}`,
    ...fields,
  });
  const look = (items: Picture[]): Run => ({
    ...run("r1", "aaa1111", "k1"),
    checks: [{ id: "screens", look: { check: "screens", base_commit: "b", base_result: "passed", items } }],
  });
  const all = picturesOf(
    look([
      variant("card/desktop/light.webm"),
      variant("card/desktop/light.png"),
      variant("card/desktop/dark.png"),
      variant("card/mobile/light.png"),
      variant("card/mobile/dark.png", { change: "same" }),
    ]),
  );
  const named = (list: { picture: Picture }[]) => list.map((p) => p.picture.name);
  const film = all.find((p) => p.picture.name === "card/desktop/light.webm")!;
  const still = all.find((p) => p.picture.name === "card/desktop/dark.png")!;

  test("a film's looks right reaches the still of its viewport and theme", () => {
    expect(named(covered(film, all, "this"))).toEqual(["card/desktop/light.webm", "card/desktop/light.png"]);
    expect(named(covered(still, all, "this"))).toEqual(["card/desktop/dark.png"]);
  });

  test("a viewport, a theme or the whole screen, only what is to look at", () => {
    expect(named(covered(still, all, "viewport"))).toEqual(["card/desktop/dark.png", "card/desktop/light.webm", "card/desktop/light.png"]);
    expect(named(covered(film, all, "theme"))).toEqual(["card/desktop/light.webm", "card/desktop/light.png", "card/mobile/light.png"]);
    expect(named(covered(still, all, "screen"))).toEqual([
      "card/desktop/dark.png",
      "card/desktop/light.webm",
      "card/desktop/light.png",
      "card/mobile/light.png",
    ]);
  });
});

describe("same inputs", () => {
  const keyed = (r: Run, inputs: string): Run => ({
    ...r,
    checks: r.checks.map((c) => ({ ...c, entry: { id: "screens", key: inputs, run: "true", stage: "look", outcome: "passed", seconds: 1 } as Check })),
  });
  const shown = (r: Run) => picturesOf(r)[0]!;
  test("a verdict holds on a picture that differs when its check read the same inputs, and marks it unstable", () => {
    const first = keyed(run("r1", "aaa1111", "k1"), "in1");
    const later = keyed(run("r2", "bbb2222", "k2"), "in1");
    const s = state([later, first]);
    expect(verdictOf(s, later, shown(later))?.run).toBe("r1");
    expect(unstableOf(s, later, shown(later))).toBe(true);
    expect(changedSince(s, later, shown(later))).toBeUndefined();
    expect(unstableOf(s, first, shown(first))).toBe(false);
  });
  test("other inputs, or a run the page does not hold, keep a picture that differs waiting", () => {
    const first = keyed(run("r1", "aaa1111", "k1"), "in1");
    const styled = keyed(run("r2", "bbb2222", "k2"), "in2");
    expect(verdictOf(state([styled, first]), styled, shown(styled))).toBeUndefined();
    const alone = keyed(run("r2", "bbb2222", "k2"), "in1");
    expect(verdictOf(state([alone]), alone, shown(alone))).toBeUndefined();
  });
});

describe("toLookAt", () => {
  const late = (max_shift: number, tail: number | null, total_after = 3): Shown =>
    picturesOf({
      ...run("r1", "aaa1111", "k1"),
      checks: [
        {
          id: "screens",
          look: {
            check: "screens",
            base_commit: "b",
            base_result: "passed",
            items: [{ ...picture, name: "a.webm", kind: "film", change: "timing", comparison: { verdict: "timing", max_shift, tail, total_before: 3, total_after } as Comparison }],
          },
        },
      ],
    })[0]!;
  test("a film only a little late waits for nobody; a much later one, or one without its tail, waits", () => {
    expect(toLookAt(late(0.3, -0.2))).toBe(false);
    expect(toLookAt(late(-0.9, 0))).toBe(false);
    expect(toLookAt(late(1.4, 0))).toBe(true);
    expect(toLookAt(late(0, 1.5))).toBe(true);
    expect(toLookAt(late(0.2, null))).toBe(true);
    expect(toLookAt(late(0.9, 0.9, 7.5))).toBe(true);
  });
});
