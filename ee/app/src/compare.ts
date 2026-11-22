// Comparing a picture of one run with the same picture of another, in the page: the worker
// runs the CLI's comparison on both files, and each pair of URLs is asked once.

import type { Answer, Ask, Promote } from "./compare.worker";
import type { Comparison, Marks } from "./proof";

let worker: Worker | undefined;
let next = 0;
const waiting = new Map<number, { done: (c: Comparison) => void; fail: (e: Error) => void }>();
const asked = new Map<string, { asking: Promise<Comparison>; id: number; ahead: boolean }>();

function started(): Worker {
  if (worker) return worker;
  const w = new Worker(new URL("./compare.worker.ts", import.meta.url), { type: "module" });
  w.onmessage = (e: MessageEvent<Answer>) => {
    const answer = e.data;
    const wait = waiting.get(answer.id);
    waiting.delete(answer.id);
    if ("json" in answer) wait?.done(JSON.parse(answer.json) as Comparison);
    else wait?.fail(new Error(answer.error));
  };
  w.onerror = () => {
    for (const wait of waiting.values()) wait.fail(new Error("the comparison stopped"));
    waiting.clear();
    worker = undefined;
  };
  worker = w;
  return w;
}

/** One side of a comparison: the file's URL and, for a film, what its check said happened in it. */
export type Side = { url: string; marks?: Marks | null };

/** The comparison of the still or film at `before` with the one at `after`, as the CLI would
 * write it, for the picture shown, or `ahead` for one a person likely shows next, compared
 * while the worker has nothing shown to compare. A pair that failed is asked again next time. */
export function compared(before: Side, after: Side, film: boolean, ahead = false): Promise<Comparison> {
  // Marks go as their JSON, which the worker hands the comparison as it is; the page's own
  // marks live in its store, whose objects a message cannot carry.
  const side = (s: Side) => ({ url: s.url, marks: s.marks ? JSON.stringify(s.marks) : null });
  const [b, a] = [side(before), side(after)];
  const key = JSON.stringify([film, b, a]);
  const known = asked.get(key);
  if (known) {
    if (known.ahead && !ahead) {
      known.ahead = false;
      started().postMessage({ promote: known.id } satisfies Promote);
    }
    return known.asking;
  }
  const id = ++next;
  const asking = new Promise<Comparison>((done, fail) => {
    waiting.set(id, { done, fail });
    started().postMessage({ id, before: b, after: a, film, ahead } satisfies Ask);
  });
  asked.set(key, { asking, id, ahead });
  asking.catch(() => asked.delete(key));
  return asking;
}
