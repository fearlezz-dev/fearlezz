import { describe, expect, test } from "bun:test";
import { apply } from "./live";
import { counts, grouped, groupings, hundredths, lanes, marksAt, parts, picturesOf, screensOf, stepAt, stillMarks } from "./pictures";
import type { Check, Comparison, Look, Pair, Picture, PictureChange, Run, Snapshot } from "./proof";

const picture = (name: string, change: PictureChange, extra: Partial<Picture> = {}): Picture => ({
  name,
  kind: "still",
  change,
  summary: "",
  before: change === "new" ? null : `looks/s/before/${name}`,
  after: change === "gone" ? null : `looks/s/after/${name}`,
  comparison: null,
  ...extra,
});
const look = (check: string, items: Picture[]): Look => ({
  check,
  base_commit: "0123456",
  base_result: "passed",
  items,
});
const run = (looks: Look[]): Run => ({
  id: "r1",
  head: "abc1234",
  base: "main",
  source: { kind: "ci", job: "proof", run_id: 1, run_attempt: 1, runner_name: "runner" },
  state: "running",
  opened: 1,
  checks: looks.map((l) => ({ id: l.check, look: l })),
});
const names = (run: Run, by: Parameters<typeof grouped>[1], filter: PictureChange | null = null) =>
  grouped(picturesOf(run), by, filter).map((g) => [g.label, g.folded, g.pictures.map((p) => p.picture.name)]);

describe("parts", () => {
  test("a name splits into its screen, viewport and theme", () => {
    expect(parts("checkout/mobile/dark.png")).toEqual({ screen: "checkout", viewport: "mobile", theme: "dark" });
    expect(parts("home/1280x720/light.png")).toEqual({ screen: "home", viewport: "1280x720", theme: "light" });
    expect(parts("settings.png")).toEqual({ screen: "settings", viewport: "", theme: "" });
    expect(parts("dark.png")).toEqual({ screen: "dark", viewport: "", theme: "dark" });
  });
});

describe("grouped", () => {
  const work = run([
    look("screens", [
      picture("home.png", "same"),
      picture("about.png", "new"),
      picture("card.png", "changed"),
      picture("old.png", "gone"),
      picture("intro.mp4", "timing", { kind: "film" }),
      picture("menu.png", "changed", { flaky: true }),
      picture("cart.png", "changed"),
    ]),
  ]);

  test("changed first, then new and gone, then timing, and same folded last", () => {
    expect(names(work, "change")).toEqual([
      ["Changed", false, ["card.png", "cart.png", "menu.png"]],
      ["New", false, ["about.png"]],
      ["Gone", false, ["old.png"]],
      ["Timing only", false, ["intro.mp4"]],
      ["Unchanged", true, ["home.png"]],
    ]);
  });

  test("a count filters to its kind, unfolded", () => {
    expect(names(work, "change", "same")).toEqual([["Unchanged", false, ["home.png"]]]);
    expect(names(work, "change", "new")).toEqual([["New", false, ["about.png"]]]);
  });

  test("a run where nothing changed shows its pictures unfolded", () => {
    expect(names(run([look("screens", [picture("a.png", "same")])]), "change")).toEqual([
      ["Unchanged", false, ["a.png"]],
    ]);
  });

  test("by a part of the name, changed first in each group, and the same pictures apart", () => {
    const themed = run([
      look("screens", [
        picture("home/light.png", "same"),
        picture("home/dark.png", "changed"),
        picture("card/light.png", "new"),
        picture("card/dark.png", "changed"),
      ]),
    ]);
    expect(groupings(picturesOf(themed))).toEqual(["change", "screen", "theme"]);
    expect(names(themed, "theme")).toEqual([
      ["dark", false, ["home/dark.png", "card/dark.png"]],
      ["light", false, ["card/light.png"]],
      ["Unchanged", true, ["home/light.png"]],
    ]);
    expect(names(themed, "screen")).toEqual([
      ["home", false, ["home/dark.png"]],
      ["card", false, ["card/dark.png", "card/light.png"]],
      ["Unchanged", true, ["home/light.png"]],
    ]);
  });

  test("pictures of several checks can be grouped by check", () => {
    const two = run([look("web", [picture("a.png", "changed")]), look("app", [picture("a.png", "new")])]);
    const pictures = picturesOf(two);
    expect(groupings(pictures)).toEqual(["change", "check"]);
    expect(new Set(pictures.map((p) => p.key)).size).toBe(2);
    expect(names(two, "check")).toEqual([
      ["web", false, ["a.png"]],
      ["app", false, ["a.png"]],
    ]);
  });

  test("counts are per kind, in order, flaky ones counted apart", () => {
    expect(counts(picturesOf(work))).toEqual([
      { change: "changed", n: 3 },
      { change: "new", n: 1 },
      { change: "gone", n: 1 },
      { change: "timing", n: 1 },
      { change: "same", n: 1 },
    ]);
  });
});

describe("live looks", () => {
  const snapshot: Snapshot = {
    seq: 1,
    work: { repository: "owner/repo", branch: "feat/x" },
    runs: [{ ...run([]), checks: [{ id: "screens" }] }],
  };
  const passed: Check = {
    id: "screens",
    run: "true",
    stage: "working",
    outcome: "passed",
    seconds: 1,
    findings_elsewhere: 0,
  };

  test("a look arrives with its check, and done's looks are the final word", () => {
    const first = look("screens", [picture("a.png", "changed")]);
    const finished = apply(snapshot, 2, "r1", { kind: "finished", check: passed, look: first });
    expect(picturesOf(finished.runs[0]!).map((p) => p.picture.name)).toEqual(["a.png"]);
    const last = look("screens", [picture("a.png", "changed", { flaky: true })]);
    const done = apply(finished, 3, "r1", { kind: "done", checks: [passed], looks: [last] });
    expect(picturesOf(done.runs[0]!)[0]!.picture.flaky).toBe(true);
    expect(done.runs[0]!.state).toBe("done");
  });
});

const pair = (before: number, after: number, before_at: number, after_at: number, px: number): Pair => ({
  before,
  after,
  before_frame: 0,
  after_frame: 0,
  before_at,
  after_at,
  px,
  boxes: px > 0 ? [[0, 0, 10, 10]] : [],
});

describe("films", () => {
  // The after film holds its second state longer, and its third state differs.
  const pairs = [pair(0, 0, 0, 0, 0), pair(1, 1, 1, 1.5, 0), pair(2, 2, 2, 3, 40)];

  test("each lane runs from state to state, differing where a pair differs", () => {
    expect(lanes(pairs, "before", 3)).toEqual([
      { start: 0, stop: 1, differs: false },
      { start: 1, stop: 2, differs: false },
      { start: 2, stop: 3, differs: true },
    ]);
    expect(lanes(pairs, "after", 4)).toEqual([
      { start: 0, stop: 1.5, differs: false },
      { start: 1.5, stop: 3, differs: false },
      { start: 3, stop: 4, differs: true },
    ]);
  });

  test("the outlines at a time are those of the state then", () => {
    const film = { pairs, size_before: [64, 48], size_after: [64, 48] } as unknown as Comparison;
    expect(marksAt(film, 0.5)).toEqual([]);
    expect(marksAt(film, 3.2)).toEqual([[0, 0, 10, 10]]);
  });

  test("a still's outlines are its moments, unless its size changed", () => {
    const moment = { before: [0, 0], after: [0, 0], box: [1, 2, 3, 4], px: 4 } as Comparison["moments"][number];
    const still = { moments: [moment], size_before: [64, 48], size_after: [64, 48] } as unknown as Comparison;
    expect(stillMarks(still)).toEqual([[1, 2, 3, 4]]);
    expect(stillMarks({ ...still, size_after: [80, 48] })).toEqual([]);
  });
});

describe("screensOf", () => {
  const variants = run([
    look("screens", [
      picture("card/mobile/dark.png", "changed"),
      picture("menu/desktop/light.png", "changed"),
      picture("card/desktop/dark.png", "changed"),
      picture("card/desktop/light.png", "changed"),
      picture("card/mobile/light.png", "changed"),
      picture("about.png", "changed"),
    ]),
  ]);

  test("one screen per name without its viewport and theme, in the order they first come", () => {
    const screens = screensOf(picturesOf(variants));
    expect(screens.map((s) => s.label)).toEqual(["card", "menu", "about"]);
  });

  test("a screen's films are among its variants, before its stills", () => {
    const both = run([
      look("screens", [
        picture("card/desktop/dark.webm", "new", { kind: "film" }),
        picture("card/desktop/dark.png", "changed"),
        picture("card/desktop/light.webm", "new", { kind: "film" }),
        picture("card/desktop/light.png", "changed"),
      ]),
    ]);
    const [card, ...others] = screensOf(picturesOf(both));
    expect(others).toEqual([]);
    expect(card!.variants.map((p) => p.picture.name)).toEqual([
      "card/desktop/light.webm",
      "card/desktop/dark.webm",
      "card/desktop/light.png",
      "card/desktop/dark.png",
    ]);
  });

  test("a screen's variants by viewport as first seen, light before dark", () => {
    const [card] = screensOf(picturesOf(variants));
    expect(card!.variants.map((p) => p.picture.name)).toEqual([
      "card/mobile/light.png",
      "card/mobile/dark.png",
      "card/desktop/light.png",
      "card/desktop/dark.png",
    ]);
    expect(card!.viewports.map((v) => [v.viewport, v.variants.map((p) => p.parts.theme)])).toEqual([
      ["mobile", ["light", "dark"]],
      ["desktop", ["light", "dark"]],
    ]);
  });

  test("the same screen in two checks is two screens", () => {
    const two = run([look("a", [picture("home/light.png", "changed")]), look("b", [picture("home/light.png", "changed")])]);
    expect(screensOf(picturesOf(two)).map((s) => s.key)).toEqual(["a/home", "b/home"]);
  });
});

describe("hundredths", () => {
  test("the clock shows the moment a note keeps, also halfway between two hundredths", () => {
    // 0.045 shown with toFixed alone reads 0.04, while the note kept 0.05.
    for (const t of [0.045, 0.125, 1.005, 2.675, 0.04]) expect(Number.parseFloat(hundredths(t).toFixed(2))).toBe(hundredths(t));
    expect(hundredths(0.045)).toBe(0.05);
  });
});

describe("stepAt", () => {
  const marks = {
    marks: [
      { name: "opened", at: 0.5 },
      { name: "timer", at: 1, hidden: true },
      { name: "pressed y", at: 2 },
    ],
    spans: [{ name: "loading", start: 0.2, end: 1.5, timing: "free" as const }],
  };
  test("names the newest shown mark or the span that started later", () => {
    expect(stepAt(marks, 0.1)).toBeUndefined();
    expect(stepAt(marks, 0.3)).toBe("loading");
    expect(stepAt(marks, 0.7)).toBe("opened");
    expect(stepAt(marks, 1.2)).toBe("opened");
    expect(stepAt(marks, 1.7)).toBe("opened");
    expect(stepAt(marks, 2.5)).toBe("pressed y");
    expect(stepAt(null, 2)).toBeUndefined();
  });
});
