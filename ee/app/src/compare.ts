// Comparing a picture of one run with the same picture of another, in the page: the worker
// runs the CLI's comparison on both files, and each pair of URLs is asked once.

import type { Answer, Ask } from "./compare.worker";
import type { Comparison } from "./proof";

let worker: Worker | undefined;
let next = 0;
const waiting = new Map<number, { done: (c: Comparison) => void; fail: (e: Error) => void }>();
const asked = new Map<string, Promise<Comparison>>();

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

/** The comparison of the still or film at `before` with the one at `after`, as the CLI would
 * write it. A pair that failed is asked again next time. */
export function compared(before: string, after: string, film: boolean): Promise<Comparison> {
  const key = `${film ? "film" : "still"} ${before} ${after}`;
  const known = asked.get(key);
  if (known) return known;
  const asking = new Promise<Comparison>((done, fail) => {
    const id = ++next;
    waiting.set(id, { done, fail });
    started().postMessage({ id, before, after, film } satisfies Ask);
  });
  asked.set(key, asking);
  asking.catch(() => asked.delete(key));
  return asking;
}
