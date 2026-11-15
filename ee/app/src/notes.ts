// Notes on the page: what a person marks should change on a picture, a region of its before,
// its after or both, with their words. The open notes of the work show on every run beside the
// picture they are on, numbered in the order they were made, the ones from an earlier run
// marked as old, until a person resolves them.

import type { Note, NoteSide, Region, Run, Snapshot } from "./proof";
import { post, refusal } from "./api";

/** One side of a pair of pictures. */
export type Side = "before" | "after";

/** A region a person is drawing or has drawn, before they wrote what should change: the side
 * drawn on, and whether it is meant for both. */
export type Draft = { side: Side; region: Region; both?: boolean };

/** A note as the page shows it beside a picture: its number there, and whether it was made on
 * another run than the one shown. */
export type Shown = { note: Note; n: number; old: boolean };

/** The open notes on picture `picture` of check `check`, oldest first and numbered, as run
 * `run` shows them. */
export function notesOn(state: Snapshot, check: string, picture: string, run: Run): Shown[] {
  return (state.notes ?? [])
    .filter((n) => n.state === "open" && n.check === check && n.picture === picture)
    .sort((a, b) => a.at - b.at)
    .map((note, k) => ({ note, n: k + 1, old: note.run !== run.id }));
}

/** Whether a note's region shows on `side`. */
export const onSide = (note: Note, side: Side) => note.side === side || note.side === "both";

/** Where an old note was made, beside who made it: an earlier push, or an earlier run. */
export function fromWhere(note: Note, run: Run): string {
  if (note.run === run.id) return "";
  if (note.head !== run.head) return `from an earlier push (${note.head.slice(0, 7)})`;
  return "from an earlier run";
}

const unit = (v: number) => Math.min(1, Math.max(0, v));

/** The region between two points of a picture, in fractions, in either order, kept on it. */
export function regionBetween(a: [number, number], b: [number, number]): Region {
  const [x0, x1] = [unit(Math.min(a[0], b[0])), unit(Math.max(a[0], b[0]))];
  const [y0, y1] = [unit(Math.min(a[1], b[1])), unit(Math.max(a[1], b[1]))];
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** Whether a drawn region is large enough to be meant, not a click. */
export const meant = (r: Region) => r.w > 0.01 && r.h > 0.01;

/** The side a note of `draft` is on: both when the person ticked it, else the side drawn on. */
export const sidesOf = (draft: Draft, both: boolean): NoteSide => (both ? "both" : draft.side);

/** Sends a note on picture `picture` of check `check` of run `run`. The error the host gave, if
 * it refused. */
export async function makeNote(
  run: string,
  ask: { check: string; picture: string; side: NoteSide; region: Region; time?: number; words: string },
): Promise<string | undefined> {
  return refusal(await post(`/api/v2/runs/${encodeURIComponent(run)}/notes`, ask));
}

/** Resolves note `id`. The error the host gave, if it refused. */
export const resolveNote = async (id: string) => refusal(await post(`/api/v2/notes/${encodeURIComponent(id)}/resolve`, {}));
