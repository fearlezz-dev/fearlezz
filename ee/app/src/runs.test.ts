import { describe, expect, test } from "bun:test";
import { picturesOf } from "./pictures";
import type { Picture, PictureVerdict, Run, Snapshot } from "./proof";
import { opensOn, stripOf } from "./runs";

const card = (after: string | null = "looks/s/after/card.png", before: string | null = "looks/s/before/card.png"): Picture => ({
  name: "card.png",
  kind: "still",
  change: "changed",
  summary: "",
  before,
  after,
  comparison: null,
});
/** A run that showed the card (unless `picture` is null) with content key `key`. */
const run = (id: string, key: string | undefined, picture: Picture | null = card(), source: Run["source"] = { kind: "ci", job: "proof", run_id: 1, run_attempt: 1, runner_name: "runner" }): Run => ({
  id,
  head: `${id}-head`,
  base: "main",
  source,
  state: "done",
  opened: 1,
  checks: [
    {
      id: "screens",
      look: { check: "screens", base_commit: "base1234567", base_result: "passed", items: picture ? [picture] : [] },
      ...(key ? { keys: { "card.png": key } } : {}),
    },
  ],
});
const verdict = (key: string, at: number, on = "r1"): PictureVerdict => ({ check: "screens", picture: "card.png", key, by: "writer", at, run: on, head: `${on}-head` });
const state = (runs: Run[], verdicts: PictureVerdict[] = []): Snapshot => ({ seq: 1, work: { repository: "owner/repo", branch: "feat/x" }, runs, verdicts });
const shown = (r: Run) => picturesOf(r)[0]!;
/** The strip as its dots' kinds and the runs each stands for. */
const dots = (s: Snapshot, now: Run) => stripOf(s, now, shown(now)).map((d) => `${d.kind}:${d.runs.map((r) => r.id).join("+")}${d.look ? ":look" : ""}`);

describe("stripOf", () => {
  test("lists the base, each earlier run that showed the picture, oldest first, then now", () => {
    const now = run("r4", "k4");
    const s = state([now, run("r3", "k3"), run("r2", undefined, null), run("r1", "k1", card(), { kind: "local", login: "claude" })]);
    expect(dots(s, now)).toEqual(["base:", "run:r1", "run:r3", "now:r4"]);
    const base = stripOf(s, now, shown(now))[0]!;
    expect([base.path, base.head]).toEqual(["looks/s/before/card.png", "base1234567"]);
    expect(stripOf(s, now, shown(now))[1]!.path).toBe("looks/s/after/card.png");
  });

  test("folds runs that showed the same picture into one dot, those like now into now", () => {
    const now = run("r5", "k3");
    const s = state([now, run("r4", "k2"), run("r3", "k3"), run("r2", "k2"), run("r1", "k1")]);
    expect(dots(s, now)).toEqual(["base:", "run:r1", "run:r4+r2", "now:r5+r3"]);
  });

  test("leaves out runs newer than the one shown, and the base when the picture is new", () => {
    const shownRun = run("r2", "k2", card("looks/s/after/card.png", null));
    const s = state([run("r3", "k3"), shownRun, run("r1", "k1")]);
    expect(dots(s, shownRun)).toEqual(["run:r1", "now:r2"]);
  });

  test("marks the run of the newest looks right on other content as your look", () => {
    const now = run("r3", "k3");
    const s = state([now, run("r2", "k2"), run("r1", "k1")], [verdict("k1", 10), verdict("k2", 20, "r2")]);
    expect(dots(s, now)).toEqual(["base:", "run:r1", "run:r2:look", "now:r3"]);
    // A looks right on what now shows is no earlier look.
    const right = state([now, run("r2", "k2"), run("r1", "k1")], [verdict("k2", 20, "r2"), verdict("k3", 30, "r3")]);
    expect(dots(right, now)).toEqual(["base:", "run:r1", "run:r2", "now:r3"]);
  });
});

describe("opensOn", () => {
  test("a picture to look again opens on your look; others on the base", () => {
    const now = run("r3", "k3");
    const again = state([now, run("r2", "k2"), run("r1", "k1")], [verdict("k2", 20, "r2")]);
    expect(opensOn(again, now, shown(now), stripOf(again, now, shown(now)))).toBe("r2");
    const fresh = state([now, run("r2", "k2"), run("r1", "k1")]);
    expect(opensOn(fresh, now, shown(now), stripOf(fresh, now, shown(now)))).toBe("base");
  });

  test("a new picture with nothing earlier opens alone", () => {
    const now = run("r1", "k1", card("looks/s/after/card.png", null));
    const s = state([now]);
    expect(opensOn(s, now, shown(now), stripOf(s, now, shown(now)))).toBe("now");
  });
});
