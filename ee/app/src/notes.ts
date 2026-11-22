// Notes on the page: what a person marks should change on a picture, a region of its before,
// its after or both, with their words. The open notes of the work show on every run beside the
// picture they are on, numbered in the order they were made, the ones from an earlier run
// marked as old, until a person resolves them.

import type { Note, NoteSide, Region, Run, Snapshot } from "./proof";
import { type Shown as Picture, title } from "./pictures";
import { post, send } from "./api";
import { hoping, pendingId, resolvedBy } from "./hopes";
import { short } from "./short";

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
  if (note.head !== run.head) return `from an earlier push (${short(note.head)})`;
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

/** What a note says when it is made: where, when, the words, and the note it was made from. */
export type Ask = { check: string; picture: string; side: NoteSide; region: Region; time?: number; words: string; link?: string };

/** Makes a note on run `run` as `by`, shown at once under an id of the page's own until the host
 * names it. The error the host gave, if it refused. */
export function makeNote(run: Run, ask: Ask, by: string): Promise<string | undefined> {
  const note: Note = { ...ask, id: pendingId(), run: run.id, head: run.head, by, at: Date.now() / 1000, state: "open" };
  return hoping(
    { kind: "note", note },
    () => post<{ note?: Note }>(`/api/v2/runs/${encodeURIComponent(run.id)}/notes`, ask),
    (answer) => (answer.note ? { kind: "note", note: answer.note } : undefined),
  );
}

/** Whether a note is still on its way to the host, so it cannot be edited or resolved yet. */
export const pending = (note: Note) => note.id.startsWith("pending-");

/** Gives note `note` new words, with `linked` on every open note linked with it too, shown at
 * once. The error the host gave, if it refused. */
export function editNote(state: Snapshot, note: Note, words: string, linked: boolean): Promise<string | undefined> {
  const notes = [note, ...(linked ? linkedWith(state, note) : [])].map((n) => ({ ...n, words }));
  return hoping({ kind: "edited", notes }, () => send("PATCH", `/api/v2/notes/${encodeURIComponent(note.id)}`, { words, linked }));
}

/** The other open notes linked with `note`: made from it, from the note it was made from, or
 * from the same first note. An edit asks whether they take the new words too. */
export function linkedWith(state: Snapshot, note: Note): Note[] {
  const group = note.link ?? note.id;
  return (state.notes ?? []).filter((n) => n.id !== note.id && n.state === "open" && (n.id === group || n.link === group));
}

/** Resolves note `note` as `by`, shown at once. The error the host gave, if it refused. */
export const resolveNote = (note: Note, by: string) =>
  hoping({ kind: "note", note: resolvedBy(note, by) }, () => post(`/api/v2/notes/${encodeURIComponent(note.id)}/resolve`, {}));

/** An open note on another variant of a screen, as a person may make it here too: where it is,
 * whether its region means the same place here (`place`: the pictures are the same size), and
 * the side it would be on, which this picture must have. */
export type Elsewhere = { note: Note; where: string; place: boolean; side: NoteSide };

const sizeOf = (p: Picture) => p.picture.comparison?.size_after ?? p.picture.comparison?.size_before;

/** The open notes on the other variants of `here`'s screen, `variants`, but those already on
 * `here` in the same words. Notes in the same words on several variants are one, naming each
 * variant, and copied from one whose region means the same place here when there is one. */
export function elsewhere(state: Snapshot, here: Picture, variants: Picture[]): Elsewhere[] {
  const open = (p: Picture) =>
    (state.notes ?? []).filter((n) => n.state === "open" && n.check === p.check && n.picture === p.picture.name).sort((a, b) => a.at - b.at);
  const said = new Set(open(here).map((n) => n.words));
  const size = sizeOf(here);
  const both = here.picture.before != null && here.picture.after != null;
  const byWords = new Map<string, Elsewhere>();
  for (const p of variants.filter((v) => v.key !== here.key)) {
    const other = sizeOf(p);
    const place = size && other ? size[0] === other[0] && size[1] === other[1] : p.parts.viewport === here.parts.viewport;
    const where = title({ screen: "", viewport: p.parts.viewport, theme: p.parts.theme }) || p.picture.name;
    for (const note of open(p).filter((n) => !said.has(n.words))) {
      const side: NoteSide = both ? note.side : here.picture.after != null ? "after" : "before";
      const known = byWords.get(note.words);
      if (!known) byWords.set(note.words, { note, where, place, side });
      else {
        known.where = `${known.where}, ${where}`;
        if (place && !known.place) Object.assign(known, { note, place, side });
      }
    }
  }
  return [...byWords.values()];
}
