import { describe, expect, test } from "bun:test";
import { FIT, MAX, panned, stepped, transform, zoomedAt } from "./zoom";

describe("zoomedAt", () => {
  test("keeps the point under the pointer where it was", () => {
    const view = zoomedAt(FIT, 2, [0.25, 0.75]);
    expect(view.s).toBe(2);
    // The point at a quarter across stays a quarter across the frame.
    expect(view.x + 0.25 * view.s).toBeCloseTo(0.25);
    expect(view.y + 0.75 * view.s).toBeCloseTo(0.75);
  });

  test("never zooms out past the whole picture nor in past the most", () => {
    expect(zoomedAt(FIT, 0.5, [0.5, 0.5])).toEqual(FIT);
    expect(zoomedAt(FIT, 100, [0.5, 0.5]).s).toBe(MAX);
  });

  test("zooming back out to the whole picture puts it back in place", () => {
    const in2 = zoomedAt(FIT, 4, [0.9, 0.1]);
    expect(zoomedAt(in2, 1 / 4, [0.2, 0.8])).toEqual(FIT);
  });
});

describe("panned", () => {
  test("moves the picture and stops at its edges", () => {
    const view = zoomedAt(FIT, 2, [0.5, 0.5]);
    const moved = panned(view, [-0.1, 0.1]);
    expect(moved.x).toBeCloseTo(view.x - 0.1);
    expect(moved.y).toBeCloseTo(view.y + 0.1);
    expect(panned(view, [5, -5])).toEqual({ s: 2, x: 0, y: -1 });
  });

  test("the whole picture does not move", () => {
    expect(panned(FIT, [0.3, 0.3])).toEqual(FIT);
  });
});

describe("stepped", () => {
  test("steps in and out around the middle, and back to the whole picture", () => {
    const view = stepped(FIT, 1);
    expect(view.s).toBe(1.5);
    expect(view.x + 0.5 * view.s).toBeCloseTo(0.5);
    expect(stepped(view, -1)).toEqual(FIT);
  });
});

test("transform places the picture from its corner", () => {
  expect(transform(FIT)).toBe("none");
  expect(transform({ s: 2, x: -0.5, y: -0.25 })).toBe("translate(-50%, -25%) scale(2)");
});
