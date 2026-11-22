import { describe, expect, test } from "bun:test";
import { picturesOf, screensOf, type Shown } from "./pictures";
import type { Picture, Run } from "./proof";
import { type Standing, moved, shape, shapeOf, standingFrom, tally, toggled } from "./board";

const variant = (name: string, fields: Partial<Picture> = {}): Picture => ({
  name,
  kind: name.endsWith(".webm") ? "film" : "still",
  change: "changed",
  summary: "",
  before: `looks/s/before/${name}`,
  after: `looks/s/after/${name}`,
  comparison: null,
  ...fields,
});
const run: Run = {
  id: "r1",
  head: "aaa1111",
  base: "main",
  source: { kind: "ci", job: "proof", run_id: 1, run_attempt: 1, runner_name: "runner" },
  state: "done",
  opened: 1,
  checks: [
    {
      id: "screens",
      look: {
        check: "screens",
        base_commit: "b",
        base_result: "passed",
        items: [
          variant("card/desktop/light.png"),
          variant("card/desktop/dark.png"),
          variant("card/tablet/light.png"),
          variant("card/tablet/dark.png"),
          variant("card/mobile/light.png"),
          variant("card/mobile/dark.png"),
        ],
      },
    },
  ],
};
const all = picturesOf(run);
const screen = screensOf(all)[0]!;
const rows = screen.viewports.map((v) => v.variants);
const at = (name: string) => all.find((p) => p.picture.name === `card/${name}.png`)!;
const here = at("desktop/light");
// Tablet light already looks right, mobile dark has two open notes.
const standing = (q: Shown): Standing => (q.key === at("tablet/light").key ? "right" : q.key === at("mobile/dark").key ? "note" : "waits");
const notes = (q: Shown) => (q.key === at("mobile/dark").key ? 2 : 0);
const names = (keys: Iterable<string>) => [...keys].map((k) => all.find((p) => p.key === k)!.picture.name.slice(5, -4)).sort();

describe("the board", () => {
  test("a shape picks what that reach covers", () => {
    expect(names(shape(here, all, "this"))).toEqual(["desktop/light"]);
    expect(names(shape(here, all, "viewport"))).toEqual(["desktop/dark", "desktop/light"]);
    expect(names(shape(here, all, "theme"))).toEqual(["desktop/light", "mobile/light", "tablet/light"]);
  });

  test("the tally marks what waits, resolves notes and counts what already looks right", () => {
    const t = tally(shape(here, all, "screen"), all, standing, notes);
    expect(t.marks.length).toBe(5);
    expect(t.notes).toBe(2);
    expect(t.already).toBe(1);
  });

  test("a tile that already looks right is never added, but any picked tile can be taken out", () => {
    const right = at("tablet/light").key;
    expect(toggled(new Set([here.key]), right, "right").has(right)).toBe(false);
    expect(toggled(new Set([here.key]), here.key, "waits").size).toBe(0);
    expect(toggled(new Set(), at("mobile/dark").key, "note").size).toBe(1);
  });

  test("picked by hand, the board still knows the shape it equals, ignoring what already looks right", () => {
    let picked = new Set([here.key]);
    picked = toggled(picked, at("mobile/light").key, "waits");
    expect(shapeOf(picked, here, all, standing)).toBe("theme");
    picked = toggled(picked, at("desktop/dark").key, "waits");
    expect(shapeOf(picked, here, all, standing)).toBeUndefined();
  });

  test("arrows move within a row and to the same place in the next", () => {
    expect(names([moved(rows, here.key, "ArrowRight")])).toEqual(["desktop/dark"]);
    expect(names([moved(rows, here.key, "ArrowLeft")])).toEqual(["desktop/light"]);
    expect(names([moved(rows, at("desktop/dark").key, "ArrowDown")])).toEqual(["tablet/dark"]);
    expect(names([moved(rows, at("mobile/dark").key, "ArrowDown")])).toEqual(["mobile/dark"]);
    expect(names([moved(rows, at("tablet/light").key, "ArrowUp")])).toEqual(["desktop/light"]);
  });

  test("an unchanged picture is never looked at", () => {
    const same = picturesOf({ ...run, checks: [{ id: "screens", look: { check: "screens", base_commit: "b", base_result: "passed", items: [variant("x/desktop/light.png", { change: "same" })] } }] })[0]!;
    expect(standingFrom(same, "none")).toBe("never");
    expect(standingFrom(here, "needs_change")).toBe("note");
  });
});
