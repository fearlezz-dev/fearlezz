// The strip of runs above a picture: the base, each earlier run that showed the picture, and
// the run shown (now), so a person can compare what now shows with what any of them showed,
// such as what it showed when they last said it looks right ("your look").

import type { Shown } from "./pictures";
import type { Marks, Run, Snapshot } from "./proof";
import { changedSince, keyOf } from "./review";

/** One dot of the strip. */
export type Dot = {
  /** `base`, `now`, or the id of the newest run it stands for. */
  key: string;
  kind: "base" | "run" | "now";
  /** The runs it stands for, newest first: runs that showed the same picture fold into one
   * dot, and those that showed what now shows, or a picture now's run found the same, into
   * now. None for the base. */
  runs: Run[];
  /** The commit it shows: the base commit, or its newest run's head. */
  head: string;
  /** The file of the picture it shows, in its newest run's files: the base's before, or a
   * run's after. */
  path: string | null;
  /** The run of the newest looks right on this picture, when that was on other content. */
  look: boolean;
  /** What the check said happened in its film, which the page times a comparison from. */
  marks: Marks | null;
};

/** The picture as `run` showed it, if it did. */
const pictureIn = (run: Run, p: Shown) => run.checks.find((c) => c.id === p.check)?.look?.items.find((i) => i.name === p.picture.name);

/** The strip for picture `p` of `run` in `state`: the base (when the picture has a before),
 * every run opened before `run` that showed the picture, oldest first, folded by what each
 * showed, then now, which also stands for the runs whose picture it found the same. */
export function stripOf(state: Snapshot, run: Run, p: Shown): Dot[] {
  const nowKey = keyOf(run, p);
  // What this run found comparing its picture with those a person said something about.
  const same = run.checks.find((c) => c.id === p.check)?.since?.[p.picture.name] ?? {};
  const now: Dot = { key: "now", kind: "now", runs: [run], head: run.head, path: p.picture.after, look: false, marks: p.picture.marks ?? null };
  // Runs are newest first; those after the one shown came before it.
  const at = state.runs.findIndex((r) => r.id === run.id);
  const earlier = at < 0 ? [] : state.runs.slice(at + 1);
  const folded = new Map<string, Dot>();
  for (const r of earlier) {
    const item = pictureIn(r, p);
    const path = item?.after;
    if (!path) continue;
    const key = keyOf(r, p);
    if (key !== undefined && (key === nowKey || same[key])) {
      now.runs.push(r);
      continue;
    }
    const fold = key === undefined ? `run:${r.id}` : `key:${key}`;
    const dot = folded.get(fold);
    if (dot) dot.runs.push(r);
    else folded.set(fold, { key: r.id, kind: "run", runs: [r], head: r.head, path, look: false, marks: item.marks ?? null });
  }
  const runs = [...folded.values()].reverse();
  const look = yourLook(state, run, p);
  const lookDot = look && (runs.find((d) => keyOf(d.runs[0]!, p) === look.key) ?? runs.find((d) => d.runs.some((r) => r.id === look.run)));
  if (lookDot) lookDot.look = true;
  const base: Dot[] = p.picture.before ? [{ key: "base", kind: "base", runs: [], head: p.look.base_commit, path: p.picture.before, look: false, marks: p.picture.before_marks ?? null }] : [];
  return [...base, ...runs, now];
}

/** The looks right on picture `p` that it changed since, by the one rule Look again uses. */
const yourLook = (state: Snapshot, run: Run, p: Shown) => changedSince(state, run, p)?.verdict;

/** The dot a picture opens compared with: your look when it is to look at again, else the
 * base, else none (`now`, the picture alone). */
export function opensOn(state: Snapshot, run: Run, p: Shown, dots: Dot[]): string {
  const look = dots.find((d) => d.look);
  if (look && changedSince(state, run, p)) return look.key;
  return dots.some((d) => d.kind === "base") ? "base" : "now";
}

/** Whether the strip is worth showing: it has a run besides the base and now. */
export const worthShowing = (dots: Dot[]) => dots.some((d) => d.kind === "run");

/** A stop on the strip as shown: a dot, or the runs between two dots kept, folded into one. */
export type Stop = { dot: Dot } | { fold: string; dots: Dot[] };

/** The strip as shown: the base, your look, the run just before now (unless `narrow`, a phone,
 * where it does not fit), now and the dot `picked` stay; each stretch of two or more runs
 * between them folds into one stop, named by its first run's key, unless `open` holds that key.
 * Most of a long history is noise, and these are the dots a person compares with. */
export function stopsOf(dots: Dot[], picked: string, open: ReadonlySet<string>, narrow = false): Stop[] {
  const kept = (d: Dot, k: number) => d.kind !== "run" || d.look || d.key === picked || (!narrow && k === dots.length - 2);
  const stops: Stop[] = [];
  let between: Dot[] = [];
  const close = () => {
    const first = between[0];
    if (first && between.length > 1 && !open.has(first.key)) stops.push({ fold: first.key, dots: between });
    else stops.push(...between.map((dot) => ({ dot })));
    between = [];
  };
  dots.forEach((d, k) => {
    if (kept(d, k)) {
      close();
      stops.push({ dot: d });
    } else between.push(d);
  });
  close();
  return stops;
}
