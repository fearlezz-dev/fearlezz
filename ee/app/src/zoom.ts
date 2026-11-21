// How far a picture on the stage is zoomed and where: one view that both sides of a comparison
// share, so the before and the after always show the same place.

/**
 * A view of a picture: `s` times as large, its top left corner at `x`, `y`, each a fraction of
 * the frame (so `-0.5` is half a frame to the left). The whole picture is `FIT`.
 */
export type View = { s: number; x: number; y: number };

export const FIT: View = { s: 1, x: 0, y: 0 };
/** The most a picture zooms in. */
export const MAX = 8;
/** How much one step of the keys or buttons zooms. */
const STEP = 1.5;

/** Keeps the picture covering its frame: never smaller, never pulled past an edge. */
function kept(view: View): View {
  const s = Math.min(MAX, Math.max(1, view.s));
  if (s === 1) return FIT;
  const edge = (v: number) => Math.min(0, Math.max(1 - s, v));
  return { s, x: edge(view.x), y: edge(view.y) };
}

/** `view` zoomed by `factor` around `at` (a fraction of the frame), which stays where it is. */
export function zoomedAt(view: View, factor: number, at: [number, number]): View {
  const s = Math.min(MAX, Math.max(1, view.s * factor));
  const r = s / view.s;
  return kept({ s, x: at[0] - (at[0] - view.x) * r, y: at[1] - (at[1] - view.y) * r });
}

/** `view` moved by `by`, a fraction of the frame. */
export function panned(view: View, by: [number, number]): View {
  return kept({ s: view.s, x: view.x + by[0], y: view.y + by[1] });
}

/** One step in (`1`) or out (`-1`), around the middle of the frame. */
export function stepped(view: View, way: 1 | -1): View {
  const s = view.s * STEP ** way;
  // A step out that lands near the whole picture lands on it.
  return zoomedAt(view, s < 1.01 ? 1 / view.s : STEP ** way, [0.5, 0.5]);
}

/** The CSS transform of the layer that holds a side's picture, its outlines and its notes. */
export function transform(view: View): string {
  if (view.s === 1) return "none";
  const pct = (v: number) => `${Math.round(v * 10000) / 100}%`;
  return `translate(${pct(view.x)}, ${pct(view.y)}) scale(${view.s})`;
}
