// How far one looks right reaches, picked on a board of the screen's variants: a row per
// viewport, a tile per picture. A shape (this picture, its viewport, its theme, the whole
// screen) picks a set at once; single tiles are then added or taken out, by pointer or by
// arrows and Space. The board itself shows what gets marked, so no reach needs a name.

import type { Shown } from "./pictures";
import { SCOPES, type Scope, covered, toLookAt } from "./review";

/** Where a tile stands before anything is said: waiting for a look, already looking right, with
 * an open note, or never to be looked at (unchanged or flaky). */
export type Standing = "waits" | "right" | "note" | "never";

/** The keys a shape picks for `p` among `variants`: what a looks right at `scope` reaches. */
export const shape = (p: Shown, variants: readonly Shown[], scope: Scope): Set<string> =>
  new Set(covered(p, variants, scope).map((q) => q.key));

/** The shape `picked` is exactly, if it is one, so the board can mark it. */
export function shapeOf(picked: ReadonlySet<string>, p: Shown, variants: readonly Shown[], standing: (q: Shown) => Standing): Scope | undefined {
  const open = (keys: ReadonlySet<string>) => [...keys].filter((k) => standingBy(k, variants, standing) !== "right").sort().join(" ");
  const now = open(picked);
  return SCOPES.find((s) => open(shape(p, variants, s)) === now);
}

const standingBy = (key: string, variants: readonly Shown[], standing: (q: Shown) => Standing) => {
  const q = variants.find((v) => v.key === key);
  return q ? standing(q) : "never";
};

/** Whether a tile can be picked: only what waits for a look or has a note. */
export const pickable = (s: Standing) => s === "waits" || s === "note";

/** `picked` with the tile `key` taken out, or added when it can be. */
export function toggled(picked: ReadonlySet<string>, key: string, s: Standing): Set<string> {
  const next = new Set(picked);
  if (next.has(key)) next.delete(key);
  else if (pickable(s)) next.add(key);
  return next;
}

/** What saying looks right on `picked` does: the pictures it marks, the open notes it resolves,
 * and how many it reaches that already look right. */
export function tally(picked: ReadonlySet<string>, variants: readonly Shown[], standing: (q: Shown) => Standing, notes: (q: Shown) => number) {
  const list = variants.filter((q) => picked.has(q.key));
  const marks = list.filter((q) => pickable(standing(q)));
  return {
    marks,
    notes: marks.reduce((n, q) => n + (standing(q) === "note" ? notes(q) : 0), 0),
    already: list.filter((q) => standing(q) === "right").length,
  };
}

/** The keys that move the cursor: the arrows, and H, J, K and L as in vim. */
export const MOVES: Record<string, "left" | "right" | "up" | "down"> = {
  ArrowLeft: "left",
  ArrowRight: "right",
  ArrowUp: "up",
  ArrowDown: "down",
  h: "left",
  l: "right",
  k: "up",
  j: "down",
};

/** The tile the cursor lands on from `at` for a moving `key` (`MOVES`), over the board's `rows`:
 * left and right within a row, up and down to the same place in the row above or below (or its
 * last). */
export function moved(rows: readonly (readonly Shown[])[], at: string, key: string): string {
  const r = rows.findIndex((row) => row.some((q) => q.key === at));
  if (r < 0) return rows[0]?.[0]?.key ?? at;
  const c = rows[r]!.findIndex((q) => q.key === at);
  const to = (row: number, col: number) => {
    const line = rows[row]!;
    return line[Math.max(0, Math.min(line.length - 1, col))]!.key;
  };
  switch (MOVES[key]) {
    case "left":
      return to(r, c - 1);
    case "right":
      return to(r, c + 1);
    case "up":
      return to(Math.max(0, r - 1), c);
    case "down":
      return to(Math.min(rows.length - 1, r + 1), c);
    default:
      return at;
  }
}

/** A tile's standing from what the review says of its picture. */
export const standingFrom = (q: Shown, review: "needs_change" | "looks_right" | "none"): Standing =>
  !toLookAt(q) ? "never" : review === "looks_right" ? "right" : review === "needs_change" ? "note" : "waits";
