import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { notesOn } from "./notes";
import { type Screen, type Shown, isFilm, title } from "./pictures";
import type { Run, Snapshot } from "./proof";
import { MOVES, type Standing, moved, shape, shapeOf, standingFrom, tally, toggled } from "./board";
import { SCOPES, type Scope, standingOf } from "./review";
import { Thumb } from "./Thumb";

/** Where the board of how far looks right reaches is open, if it is: beside its arrow, or
 * floating on the screen for a held Y, since Looks right may be out of view then. */
export type Menu = false | "here" | "float";

/** The board of a screen's variants that a looks right reaches: a row per viewport, a tile per
 * picture showing where it stands, what is picked lit. It opens on this picture; 1 to 4 pick a
 * shape, arrows or H, J, K and L and Space (or a press and a sweep) pick tiles, and Y or Enter says it. */
export function Reach(props: {
  shown: Shown;
  variants: Shown[];
  viewports: Screen["viewports"];
  state: Snapshot;
  run: Run;
  files: string;
  menu: Menu;
  onMenu: (open: Menu) => void;
  /** A tile the cursor or a press lands on, to show behind the board. */
  onFocus: (key: string) => void;
  onSay: (list: Shown[]) => void;
}) {
  const standing = (q: Shown): Standing => standingFrom(q, standingOf(props.state, props.run, q));
  const notes = (q: Shown) => notesOn(props.state, q.check, q.picture.name, props.run).length;
  const byKey = (key: string) => props.variants.find((q) => q.key === key);
  const [picked, setPicked] = createSignal<ReadonlySet<string>>(new Set());
  const [cursor, setCursor] = createSignal("");
  // Each time it opens, and for another screen, it starts on what a plain Y says: this picture
  // (a film with its still). Moving to another picture of the screen keeps what is picked.
  const screen = () => props.variants.map((q) => q.key).join(" ");
  createEffect(
    on([() => !!props.menu, screen], ([open, now], was) => {
      if (!open || (was?.[0] && was[1] === now)) return;
      setPicked(shape(props.shown, props.variants, "this"));
    }),
  );
  createEffect(() => setCursor(props.shown.key));
  const focus = (key: string) => {
    setCursor(key);
    props.onFocus(key);
  };
  const rows = () => props.viewports.map((v) => v.variants);
  const told = createMemo(() => tally(picked(), props.variants, standing, notes));
  const now = createMemo(() => shapeOf(picked(), props.shown, props.variants, standing));
  // Each shape, and whether it reaches more than this picture does (a viewport or theme the
  // pictures do not name reaches nothing of its own).
  const shapes = createMemo(() => {
    const here = shape(props.shown, props.variants, "this").size;
    const { viewport, theme } = props.shown.parts;
    return SCOPES.map((scope) => {
      const keys = shape(props.shown, props.variants, scope);
      const named = scope === "viewport" ? !!viewport : scope === "theme" ? !!theme : true;
      const label =
        scope === "this" ? "This picture" : scope === "viewport" ? `All of ${viewport}` : scope === "theme" ? `All ${theme}` : "The whole screen";
      return { scope, keys, label, open: scope === "this" || (named && keys.size > here) };
    });
  });
  const pick = (scope: Scope) => {
    const s = shapes().find((x) => x.scope === scope);
    if (s?.open) setPicked(s.keys);
  };
  const flip = (key: string) => {
    const q = byKey(key);
    if (q) setPicked(toggled(picked(), key, standing(q)));
  };
  // A press on a tile flips it, and a sweep with the button down paints the same way.
  let painting: boolean | undefined;
  const down = (key: string) => {
    painting = !picked().has(key);
    focus(key);
    flip(key);
  };
  const over = (key: string) => {
    if (painting === undefined || picked().has(key) === painting) return;
    focus(key);
    flip(key);
  };
  const say = () => {
    const list = told().marks;
    if (list.length) props.onSay(list);
  };

  onMount(() => {
    // While it is open the board takes these keys before the page does.
    const keys = (e: KeyboardEvent) => {
      if (!props.menu) return;
      const target = e.target as HTMLElement | null;
      if (e.metaKey || e.ctrlKey || e.altKey || target?.closest("input, textarea, select, [contenteditable]")) return;
      if (/^[1-4]$/.test(e.key)) pick(SCOPES[Number(e.key) - 1]!);
      else if (e.key in MOVES) focus(moved(rows(), cursor(), e.key));
      else if (e.key === " ") flip(cursor());
      // A Y still held from opening the board repeats; only a new press says it.
      else if (e.key === "Enter" || e.key.toLowerCase() === "y") {
        if (!e.repeat) say();
      } else if (e.key === "Escape") props.onMenu(false);
      else return;
      e.preventDefault();
      e.stopImmediatePropagation();
    };
    const stop = () => (painting = undefined);
    document.addEventListener("keydown", keys, true);
    document.addEventListener("pointerup", stop);
    onCleanup(() => {
      document.removeEventListener("keydown", keys, true);
      document.removeEventListener("pointerup", stop);
    });
  });

  const tag = (q: Shown) => {
    const s = standing(q);
    if (picked().has(q.key)) return s === "right" ? "already right" : s === "note" ? "resolves note" : "to mark";
    return s === "right" ? "looks right" : s === "note" ? "has a note" : s === "never" ? "unchanged" : q.key === props.shown.key ? "on the stage" : "waits";
  };
  const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

  return (
    <div class="say-menu reach" data-say-menu role="dialog" aria-label="Looks right on" hidden={!props.menu} data-float={props.menu === "float" ? "" : undefined}>
      <p class="say-head">
        Looks right on <span class="esc">Esc closes</span>
      </p>
      <div class="reach-body">
        <div class="reach-board" role="group" aria-label="Variants">
          <For each={props.viewports}>
            {(v) => (
              <div class="reach-row">
                <Show when={v.viewport}>
                  <span class="vp">{v.viewport}</span>
                </Show>
                <For each={v.variants}>
                  {(q) => (
                    <button
                      type="button"
                      class={`reach-tile theme-${q.parts.theme || "none"}`}
                      data-reach-tile={q.key}
                      data-reach-standing={standing(q)}
                      data-here={q.key === props.shown.key ? "" : undefined}
                      data-cursor={cursor() === q.key ? "" : undefined}
                      aria-pressed={picked().has(q.key)}
                      disabled={standing(q) === "never"}
                      aria-label={`${title(q.parts)}${isFilm(q.picture) ? ", film" : ""}: ${tag(q)}`}
                      title={`${[q.parts.viewport, q.parts.theme].filter(Boolean).join(", ")}${isFilm(q.picture) ? ", film" : ""}`}
                      onPointerDown={(e) => {
                        e.preventDefault();
                        down(q.key);
                      }}
                      onPointerEnter={() => over(q.key)}
                      // A click with no pointer behind it (assistive technology) flips it too.
                      onClick={(e) => e.detail === 0 && flip(q.key)}
                    >
                      <span class="reach-shot">
                        <Thumb shown={q} files={props.files} />
                        <Show when={isFilm(q.picture)}>
                          <span class="film-mark" aria-hidden="true" />
                        </Show>
                        <span class="reach-mark" aria-hidden="true" />
                      </span>
                      <span class="reach-tag">{tag(q)}</span>
                    </button>
                  )}
                </For>
              </div>
            )}
          </For>
        </div>
        <div class="reach-side">
          <div class="reach-shapes">
            <For each={shapes()}>
              {(s, k) => (
                <button
                  type="button"
                  class="reach-shape"
                  data-say-scope={s.scope}
                  aria-pressed={now() === s.scope}
                  aria-label={s.label}
                  title={s.label}
                  disabled={!s.open}
                  onClick={() => pick(s.scope)}
                >
                  <span class="reach-mini" aria-hidden="true">
                    <For each={rows()}>
                      {(row) => (
                        <span class="reach-mini-row">
                          <For each={row}>
                            {(q) => (
                              <b
                                classList={{
                                  dark: q.parts.theme === "dark",
                                  lit: s.keys.has(q.key) && standing(q) !== "right" && standing(q) !== "never",
                                  right: standing(q) === "right",
                                  note: standing(q) === "note",
                                  here: q.key === props.shown.key,
                                }}
                              />
                            )}
                          </For>
                        </span>
                      )}
                    </For>
                  </span>
                  <kbd>{k() + 1}</kbd>
                </button>
              )}
            </For>
          </div>
          <p class="reach-tally" data-reach-tally>
            <Show when={told().marks.length} fallback={<span class="reach-none">Nothing new to mark</span>}>
              <span class="reach-marks">Marks {told().marks.length}</span>
            </Show>
            <Show when={told().notes}>
              <span class="reach-notes">resolves {plural(told().notes, "note")}</span>
            </Show>
            <Show when={told().already}>
              <span class="reach-none">{told().already} already right</span>
            </Show>
          </p>
          <button type="button" class="btn ok" data-say-reach disabled={!told().marks.length} onClick={say}>
            Looks right <kbd>Y</kbd>
          </button>
        </div>
      </div>
      <p class="reach-keys">
        <kbd>1</kbd>–<kbd>4</kbd> shape · <kbd>←↑↓→</kbd> or <kbd>hjkl</kbd> <kbd>Space</kbd> pick · <kbd>Y</kbd> says it
      </p>
    </div>
  );
}
