// Two texts line by line, as the page shows any diff: a part of the config before and after,
// or a text of a look.

import { For, Show } from "solid-js";
import { type Fold, type Line, diff, folded, sent } from "./diff";
import type { DiffLine } from "./proof";

/** `was` and `now` line by line, the words that differ marked. With `context`, runs of kept
 * lines longer than that on each side of a change fold to a count, for a long text. A text's
 * diff the bundle carries (`lines`) shows as it is, already folded. */
export function DiffLines(
  props: { was: string | null | undefined; now: string | null | undefined; context?: number } | { lines: DiffLine[] },
) {
  const lines = (): (Line | Fold)[] => {
    if ("lines" in props) return sent(props.lines);
    const all = diff(props.was, props.now);
    return props.context == null ? all : folded(all, props.context);
  };
  return (
    <pre class="diff-lines" data-diff>
      <For each={lines()}>
        {(line) => (
          <Show
            when={"kind" in line ? line : undefined}
            fallback={
              <span class="line fold" data-line="fold">
                {(line as Fold).count} unchanged {(line as Fold).count === 1 ? "line" : "lines"}
              </span>
            }
          >
            {(l) => (
              <span class={`line ${l().kind}`} data-line={l().kind}>
                <span class="sign" aria-hidden="true">
                  {l().kind === "same" ? " " : l().kind === "was" ? "-" : "+"}
                </span>
                <For each={l().pieces}>{(p) => (p.differs ? <mark>{p.text}</mark> : p.text)}</For>
              </span>
            )}
          </Show>
        )}
      </For>
    </pre>
  );
}
