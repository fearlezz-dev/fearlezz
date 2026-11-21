// The strip of runs above a picture: a dot for the base, for each earlier run that showed it,
// and for now. Pressing a dot compares what that run showed (left) with now (right). The run of
// the person's last looks right, "your look", is drawn in the Look again colour, with its line
// to now, since that stretch is what changed since they looked.

import { For } from "solid-js";
import type { Dot } from "./runs";
import { short } from "./short";

/** What a dot is called on the strip. */
export function dotName(d: Dot): string {
  if (d.kind === "base") return "Base";
  if (d.kind === "now") return "Now";
  if (d.look) return "Your look";
  return d.runs[0]?.source.kind === "local" ? "Local run" : "Run";
}

/** A dot's full description, for its title. */
function described(d: Dot): string {
  if (d.kind === "base") return `The base commit, ${short(d.head)}, as CI compared it`;
  const runs = d.runs.map((r) => `${r.source.kind === "local" ? `${r.source.login || "a developer"}'s local run` : "CI run"} at ${short(r.head)}`);
  const same = runs.length > 1 ? `, which ${runs.length} runs showed alike: ${runs.join(", ")}` : `: ${runs[0] ?? ""}`;
  if (d.kind === "now") return `Now, the run shown${same}`;
  return `${d.look ? "Your look, where it last looked right" : "An earlier run"}${same}. Compare it with now`;
}

export function Strip(props: { dots: Dot[]; picked: string; onPick: (key: string) => void }) {
  const at = (key: string) => props.dots.findIndex((d) => d.key === key);
  const look = () => props.dots.findIndex((d) => d.look);
  return (
    <nav
      class="strip"
      data-strip
      aria-label="Compare now with"
      // Where a phone scrolls it sideways, it opens at its end, on now and the runs before it.
      ref={(el) => queueMicrotask(() => (el.scrollLeft = el.scrollWidth))}
    >
      <ol>
        <For each={props.dots}>
          {(d, k) => (
            <li
              class={d.kind}
              data-dot={d.key}
              data-kind={d.kind}
              data-look={d.look ? "" : undefined}
              // The line into this dot: part of the pair shown, and of the stretch since your look.
              data-in-pair={k() > at(props.picked) && at(props.picked) >= 0 ? "" : undefined}
              data-since-look={look() >= 0 && k() > look() ? "" : undefined}
            >
              <button
                type="button"
                aria-pressed={props.picked === d.key || d.kind === "now"}
                disabled={d.kind === "now"}
                title={described(d)}
                onClick={() => props.onPick(d.key)}
              >
                <span class="pip" aria-hidden="true" />
                <span class="dot-name">
                  {dotName(d)}
                  {d.runs.length > 1 ? <span class="folded num"> ×{d.runs.length}</span> : ""}
                </span>
                <span class="dot-head num">{short(d.head)}</span>
              </button>
            </li>
          )}
        </For>
      </ol>
    </nav>
  );
}
