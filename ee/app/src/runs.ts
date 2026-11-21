// The strip of runs above a picture: the base, each earlier run that showed the picture, and
// the run shown (now), so a person can compare what now shows with what any of them showed,
// such as what it showed when they last said it looks right ("your look").

import type { Shown } from "./pictures";
import type { Run, Snapshot } from "./proof";
import { changedSince, keyOf } from "./review";

/** One dot of the strip. */
export type Dot = {
  /** `base`, `now`, or the id of the newest run it stands for. */
  key: string;
  kind: "base" | "run" | "now";
  /** The runs it stands for, newest first: runs that showed the same picture fold into one
   * dot, and those that showed what now shows into now. None for the base. */
  runs: Run[];
  /** The commit it shows: the base commit, or its newest run's head. */
  head: string;
  /** The file of the picture it shows, in its newest run's files: the base's before, or a
   * run's after. */
  path: string | null;
  /** The run of the newest looks right on this picture, when that was on other content. */
  look: boolean;
};

/** The picture as `run` showed it, if it did. */
const pictureIn = (run: Run, p: Shown) => run.checks.find((c) => c.id === p.check)?.look?.items.find((i) => i.name === p.picture.name);

/** The strip for picture `p` of `run` in `state`: the base (when the picture has a before),
 * every run opened before `run` that showed the picture, oldest first, folded by what each
 * showed, then now. */
export function stripOf(state: Snapshot, run: Run, p: Shown): Dot[] {
  const nowKey = keyOf(run, p);
  const now: Dot = { key: "now", kind: "now", runs: [run], head: run.head, path: p.picture.after, look: false };
  // Runs are newest first; those after the one shown came before it.
  const at = state.runs.findIndex((r) => r.id === run.id);
  const earlier = at < 0 ? [] : state.runs.slice(at + 1);
  const folded = new Map<string, Dot>();
  for (const r of earlier) {
    const path = pictureIn(r, p)?.after;
    if (!path) continue;
    const key = keyOf(r, p);
    if (key !== undefined && key === nowKey) {
      now.runs.push(r);
      continue;
    }
    const fold = key === undefined ? `run:${r.id}` : `key:${key}`;
    const dot = folded.get(fold);
    if (dot) dot.runs.push(r);
    else folded.set(fold, { key: r.id, kind: "run", runs: [r], head: r.head, path, look: false });
  }
  const runs = [...folded.values()].reverse();
  const look = yourLook(state, run, p);
  const lookDot = look && (runs.find((d) => keyOf(d.runs[0]!, p) === look.key) ?? runs.find((d) => d.runs.some((r) => r.id === look.run)));
  if (lookDot) lookDot.look = true;
  const base: Dot[] = p.picture.before ? [{ key: "base", kind: "base", runs: [], head: p.look.base_commit, path: p.picture.before, look: false }] : [];
  return [...base, ...runs, now];
}

/** The newest looks right on picture `p`, when it was on other content than `run` shows. */
function yourLook(state: Snapshot, run: Run, p: Shown) {
  const newest = (state.verdicts ?? []).filter((v) => v.check === p.check && v.picture === p.picture.name).sort((a, b) => b.at - a.at)[0];
  return newest && newest.key !== keyOf(run, p) ? newest : undefined;
}

/** The dot a picture opens compared with: your look when it is to look at again, else the
 * base, else none (`now`, the picture alone). */
export function opensOn(state: Snapshot, run: Run, p: Shown, dots: Dot[]): string {
  const look = dots.find((d) => d.look);
  if (look && changedSince(state, run, p)) return look.key;
  return dots.some((d) => d.kind === "base") ? "base" : "now";
}

/** Whether the strip is worth showing: it has a run besides the base and now. */
export const worthShowing = (dots: Dot[]) => dots.some((d) => d.kind === "run");
