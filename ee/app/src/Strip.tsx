// The strip of runs above a picture: a dot for the base, for each earlier run that showed it,
// and for now. Pressing a dot compares what that run showed (left) with now (right). The run of
// the person's last looks right, "your look", is drawn in the Look again colour, with its line
// to now, since that stretch is what changed since they looked. A long history folds: the runs
// between the dots kept (`stopsOf`) show as one stop, which opens them in place.

import { For, createEffect, createSignal, on, onCleanup } from "solid-js";
import { type Dot, stopsOf } from "./runs";
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
  // The folds opened, by their first run; another picture starts folded again.
  const [open, setOpen] = createSignal<ReadonlySet<string>>(new Set());
  createEffect(on(() => props.dots, () => setOpen(new Set<string>()), { defer: true }));
  // A phone's width fits the base, your look, now and two folds; the page's phone layout starts
  // at 700px (page.css).
  const phone = window.matchMedia("(max-width: 700px)");
  const [narrow, setNarrow] = createSignal(phone.matches);
  const follow = () => setNarrow(phone.matches);
  phone.addEventListener("change", follow);
  onCleanup(() => phone.removeEventListener("change", follow));
  const stops = () => stopsOf(props.dots, props.picked, open(), narrow());
  // The line into a stop: part of the pair shown, and of the stretch since your look.
  const lines = (first: Dot) => {
    const k = at(first.key);
    return {
      "data-in-pair": k > at(props.picked) && at(props.picked) >= 0 ? "" : undefined,
      "data-since-look": look() >= 0 && k > look() ? "" : undefined,
    };
  };
  return (
    <nav
      class="strip"
      data-strip
      aria-label="Compare now with"
      // Where a phone scrolls it sideways, it opens at its end, on now and the runs before it.
      ref={(el) => queueMicrotask(() => (el.scrollLeft = el.scrollWidth))}
    >
      <span class="strip-keys" data-strip-keys aria-hidden="true" title="[ and ] step to the older and newer run, B to the base">
        <kbd>[</kbd>
        <kbd>]</kbd>
      </span>
      <ol>
        <For each={stops()}>
          {(stop) =>
            "fold" in stop ? (
              <li class="fold" data-fold={stop.fold} {...lines(stop.dots[0]!)}>
                <button
                  type="button"
                  aria-label={`Show ${stop.dots.length} more runs`}
                  title={stop.dots.map((d) => `${dotName(d)} ${short(d.head)}`).join(", ")}
                  onClick={() => setOpen((o) => new Set([...o, stop.fold]))}
                >
                  <span class="num">+{stop.dots.length}</span>
                </button>
              </li>
            ) : (
              <li class={stop.dot.kind} data-dot={stop.dot.key} data-kind={stop.dot.kind} data-look={stop.dot.look ? "" : undefined} {...lines(stop.dot)}>
                <button
                  type="button"
                  aria-pressed={props.picked === stop.dot.key || stop.dot.kind === "now"}
                  disabled={stop.dot.kind === "now"}
                  title={described(stop.dot)}
                  onClick={() => props.onPick(stop.dot.key)}
                >
                  <span class="pip" aria-hidden="true" />
                  <span class="dot-name">
                    {dotName(stop.dot)}
                    {stop.dot.runs.length > 1 ? <span class="folded num"> ×{stop.dot.runs.length}</span> : ""}
                  </span>
                  <span class="dot-head num">{short(stop.dot.head)}</span>
                </button>
              </li>
            )
          }
        </For>
      </ol>
    </nav>
  );
}
