// A run's pictures: counts that filter, grouping by the parts of their names, every picture in
// the order that directs attention, the one on the stage, the notes on it and the verdict. j
// and k step between pictures, 1 to 5 pick the way to compare.

import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, on, onCleanup, onMount, untrack } from "solid-js";
import { fitting } from "./fit";
import { type Menu, Reach } from "./Reach";
import { Thumb } from "./Thumb";
import { fileUrl, grantOf, granted } from "./grant";
import {
  GROUP_WORDS,
  type Group,
  type GroupBy,
  type Shown,
  WORDS,
  counts,
  grouped,
  hundredths,
  groupings,
  isFilm,
  isText,
  picturesOf,
  type Screen,
  screensOf,
  title,
} from "./pictures";
import { type Draft, type Elsewhere, type Shown as Noted, editNote, elsewhere, fromWhere, linkedWith, makeNote, notesOn, pending, resolveNote, sidesOf } from "./notes";
import type { Note, PictureChange, Run, Snapshot } from "./proof";
import { type Looker, canSay, carried, changedSince, covered, localBy, maySignIn, onlyViews, signIn, signedIn, standingOf, toLookAt, verdict as say, verdictOf, withdraw } from "./review";
import { type Against, MODES, type Mode, type Noting, Stage, stepSpeed } from "./Stage";
import { opensOn, stripOf, worthShowing } from "./runs";
import { Strip, dotName } from "./Strip";
import { compared } from "./compare";
import { FIT, type View } from "./zoom";
import { ago } from "./ago";
import { prefetch, preload } from "./preload";
import { Icon } from "./icons";
import { short } from "./short";

// The way to compare and the grouping stay as a person moves between pictures and runs.
const [mode, setMode] = createSignal<Mode>("side");
const [by, setBy] = createSignal<GroupBy>("change");

/** Grows a textarea with its words from three lines to six, then it scrolls inside; CSS does
 * it where the browser knows `field-sizing`, this elsewhere. */
function grows(el: HTMLTextAreaElement) {
  if (CSS.supports("field-sizing", "content")) return;
  const fit = () => {
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight + el.offsetHeight - el.clientHeight}px`;
  };
  el.addEventListener("input", fit);
  queueMicrotask(fit);
}

/** How long Y is held before it opens the choice of how far looks right reaches. */
const HOLD_MS = 350;

/** How many pictures after the one shown are fetched ahead. */
const AHEAD = 3;

export function PicturesPanel(props: { run: Run; state: Snapshot; viewer: Looker | undefined }) {
  const files = grantOf(() => props.run.id);
  const pictures = createMemo(() => picturesOf(props.run));
  const [filter, setFilter] = createSignal<PictureChange | null>(null);
  const [unfolded, setUnfolded] = createSignal<ReadonlySet<string>>(new Set());
  const [picked, setPicked] = createSignal<string | null>(null);
  const options = createMemo(() => groupings(pictures()));
  const grouping = () => (options().includes(by()) ? by() : "change");
  const groups = createMemo(() => grouped(pictures(), grouping(), filter()));
  const isOpen = (g: Group) => !g.folded || unfolded().has(g.key);
  // Each group's pictures as screens, a screen's variants together.
  const screens = createMemo(() => new Map(groups().map((g) => [g.key, screensOf(g.pictures)])));
  // The order j and k step through: what shows, screen by screen, folded groups left out until
  // opened.
  const order = createMemo(() => groups().flatMap((g) => (isOpen(g) ? (screens().get(g.key) ?? []).flatMap((s) => s.variants) : [])));
  // What the page opens on: the first picture to look at again, since it changed after a look
  // or a note, then the first that waits for a look, else the first.
  const opening = () => {
    const list = order().filter(toLookAt);
    const again = list.find((p) => changedSince(props.state, props.run, p));
    return again ?? list.find((p) => standingOf(props.state, props.run, p) === "none") ?? order()[0];
  };
  const current = createMemo<Shown | undefined>(() => {
    const key = picked();
    return order().find((p) => p.key === key) ?? pictures().find((p) => p.key === key) ?? opening();
  });
  // Opened, it stays there while what waits changes, until a person moves on.
  createEffect(() => {
    const shown = current();
    if (shown && untrack(picked) !== shown.key) setPicked(shown.key);
  });
  const currentKey = createMemo(() => current()?.key);
  // The strip of runs above the picture, and the run its left side shows: the one picked for
  // this picture, else what it opens on (your look for a picture to look again, else the base).
  const dots = createMemo(() => {
    const p = current();
    return p && !isText(p.picture) && p.picture.after ? stripOf(props.state, props.run, p) : [];
  });
  const [pickedDots, setPickedDots] = createSignal<Record<string, string>>({});
  const dotKey = createMemo(() => {
    const p = current();
    if (!p) return "now";
    const picked = pickedDots()[p.key];
    return dots().some((d) => d.key === picked) ? picked! : opensOn(props.state, props.run, p, dots());
  });
  const dot = () => dots().find((d) => d.key === dotKey());
  const pickDot = (key: string) => {
    const p = current();
    if (p) setPickedDots((was) => ({ ...was, [p.key]: key }));
  };
  // An earlier run's files come under that run's own grant.
  const leftFiles = grantOf(() => {
    const d = dot();
    return d?.kind === "run" ? d.runs[0]!.id : props.run.id;
  });
  const pairUrls = createMemo(() => {
    const d = dot();
    const p = current();
    const left = leftFiles();
    const right = files();
    if (d?.kind !== "run" || !d.path || !p?.picture.after || !left || !right) return undefined;
    return {
      before: fileUrl(left, d.path),
      after: fileUrl(right, p.picture.after),
      film: isFilm(p.picture),
      marks: [d.marks, p.picture.marks ?? null] as const,
    };
    // The same files are the same pair, so a live event that redraws the run compares nothing again.
  }, undefined, { equals: (a, b) => a?.before === b?.before && a?.after === b?.after && a?.film === b?.film });
  // Its comparison with now, made in the page.
  const [pair] = createResource(pairUrls, (u) => compared({ url: u.before, marks: u.marks[0] }, { url: u.after, marks: u.marks[1] }, u.film));
  const against = createMemo<Against | undefined>(() => {
    const d = dot();
    if (d?.kind !== "run") return undefined;
    const urls = pairUrls();
    const error = pair.state === "errored" ? (pair.error instanceof Error ? pair.error.message : String(pair.error)) : undefined;
    return {
      url: urls?.before ?? "",
      word: dotName(d),
      label: `${dotName(d)}, ${short(d.head)}`,
      look: d.look,
      marks: d.marks,
      said: (() => {
        const p = current();
        const v = d.look && p ? changedSince(props.state, props.run, p)?.verdict : undefined;
        return v ? `${v.by}'s look at ${short(v.head)}` : undefined;
      })(),
      comparison: urls && pair.state === "ready" ? pair() : undefined,
      error,
    };
  });
  // The note being drawn, and where a film's clock is, for the picture on the stage.
  const [draft, setDraft] = createSignal<Draft>();
  const [composing, setComposing] = createSignal(false);
  const [time, setTime] = createSignal<number>();
  createEffect(
    on(currentKey, () => {
      setDraft(undefined);
      setComposing(false);
    }),
  );
  // The zoom stays on the next picture of the same size, such as the screen's other theme,
  // so the same place can be compared; another size shows whole.
  const [view, setView] = createSignal<View>(FIT);
  const sizeOf = createMemo(() => current()?.picture.comparison?.size_after.join("x"));
  createEffect(on(sizeOf, () => setView(FIT), { defer: true }));
  const [panning, setPanning] = createSignal(false);
  // The stage and its review on the whole screen: the browser's full screen where it has one,
  // and on the page's whole window either way (a phone has no full screen for a page).
  const [full, setFull] = createSignal(false);
  let table: HTMLDivElement | undefined;
  const toggleFull = () => {
    const on = !full();
    setFull(on);
    // The browser's full screen takes Esc for itself; locked, a press reaches the page (which
    // closes what is open first) and holding it still leaves, where the browser can lock keys.
    if (on && document.fullscreenEnabled && !document.fullscreenElement)
      table
        ?.requestFullscreen()
        .then(() => (navigator as Navigator & { keyboard?: { lock?: (keys: string[]) => Promise<void> } }).keyboard?.lock?.(["Escape"]))
        .catch(() => {});
    if (!on && document.fullscreenElement) document.exitFullscreen().catch(() => {});
  };
  // In full screen the pictures take what the window has left, so it never scrolls.
  createEffect(() => {
    if (!full() || !table) return;
    onCleanup(fitting(table));
  });
  const notes = createMemo<Noted[]>(() => {
    const p = current();
    return p ? notesOn(props.state, p.check, p.picture.name, props.run) : [];
  });
  // The open notes on the other variants of the screen on the stage, to make here too.
  const fromVariants = createMemo<Elsewhere[]>(() => {
    const p = current();
    if (!p) return [];
    const screen = screensOf(pictures()).find((s) => s.variants.some((v) => v.key === p.key));
    return elsewhere(props.state, p, screen?.variants ?? []);
  });
  // Notes may be made on any run, a local one too: that is how they reach the agent early.
  const canNote = () => canSay(props.viewer);
  const noting: Noting = {
    get notes() {
      return notes();
    },
    get draft() {
      return draft();
    },
    get can() {
      return canNote() && mode() === "side";
    },
    // Notes are on the base's before and now's after; an earlier run's picture takes none.
    get only() {
      return against() ? "after" : undefined;
    },
    onDraft: (next, done) => {
      setDraft(next && draft()?.both ? { ...next, both: true } : next);
      setComposing(!!next && (done || composing()));
      if (done) queueMicrotask(() => document.querySelector<HTMLElement>("textarea[data-words]")?.focus());
    },
  };
  const checks = createMemo(() => new Set(pictures().map((p) => p.check)).size);
  // While the next picture loads, the stage keeps the height it had, so the page never gets
  // shorter under a person stepping through and scrolls them away from the picture.
  const [hold, setHold] = createSignal<number>();
  let stageBox: HTMLDivElement | undefined;
  let release: ReturnType<typeof setTimeout> | undefined;
  const holdStage = () => {
    setHold(stageBox?.offsetHeight || undefined);
    clearTimeout(release);
    release = setTimeout(() => setHold(undefined), 2000);
    // A text loads nothing: once its stage is drawn, there is nothing to wait for.
    requestAnimationFrame(() => {
      if (stageBox && !stageBox.querySelector("img, video")) loaded();
    });
  };
  const loaded = () => {
    clearTimeout(release);
    release = setTimeout(() => setHold(undefined), 50);
  };
  onCleanup(() => clearTimeout(release));

  const step = (by: number) => {
    const list = order();
    const at = list.findIndex((p) => p.key === current()?.key);
    const next = list[Math.max(0, Math.min(list.length - 1, at + by))];
    if (next) pick(next.key);
  };
  const pick = (key: string) => {
    if (key === current()?.key) return;
    holdStage();
    setPicked(key);
  };
  // The same screen in its other theme or viewport, keeping the other part where it can.
  const swap = (part: "theme" | "viewport") => {
    const now = current();
    if (!now) return;
    const screen = [...screens().values()].flat().find((s) => s.variants.some((p) => p.key === now.key));
    const other = part === "theme" ? "viewport" : "theme";
    const others = (screen?.variants ?? []).filter((p) => p.parts[part] !== now.parts[part]);
    const next = others.find((p) => p.parts[other] === now.parts[other]) ?? others[0];
    if (next) pick(next.key);
  };
  const [keysShown, setKeysShown] = createSignal(false);
  // The choice of how far a looks right reaches: beside its arrow when that was pressed, or
  // floating on the screen when Y was held, since Looks right may be out of view then.
  const [menu, setMenu] = createSignal<Menu>(false);
  // The screen of the picture shown, with every variant of it.
  const screenOf = () => [...screens().values()].flat().find((s) => s.variants.some((p) => p.key === current()?.key));
  // The next picture that still waits for a verdict, after this one first.
  const waiting = () => {
    const list = order();
    const at = list.findIndex((p) => p.key === current()?.key);
    // What waits for a look, and what changed since its note, to be checked against it.
    const waits = (p: Shown) =>
      p.key !== current()?.key &&
      toLookAt(p) &&
      (standingOf(props.state, props.run, p) === "none" || !!changedSince(props.state, props.run, p)?.notes.length);
    return [...list.slice(at + 1), ...list.slice(0, Math.max(0, at))].find(waits);
  };
  // After a verdict, on to it.
  const onward = () => {
    const next = waiting();
    if (next) pick(next.key);
  };
  // The pictures a person most likely shows next, fetched ahead: the one a verdict moves on
  // to, then the next ones in order, across screens; films too, which take longest.
  createEffect(() => {
    const list = order();
    const at = list.findIndex((p) => p.key === current()?.key);
    const next = [waiting(), ...list.slice(at + 1, at + 1 + AHEAD)].filter((p): p is Shown => !!p && !isText(p.picture));
    const grant = files();
    if (!grant) return;
    const urls = (p: Shown) => [p.picture.before, p.picture.after].filter((path): path is string => !!path).map((path) => fileUrl(grant, path));
    preload(next.filter((p) => !isFilm(p.picture)).flatMap(urls));
    prefetch(next.filter((p) => isFilm(p.picture)).flatMap(urls));
    // And what each opens compared with when that is an earlier run, such as your look: its
    // file fetched and the page's comparison made ahead, so stepping to it shows it at once.
    const state = props.state;
    const run = props.run;
    let stale = false;
    onCleanup(() => {
      stale = true;
    });
    for (const p of next) {
      const after = p.picture.after;
      const dots = after ? stripOf(state, run, p) : [];
      const d = dots.find((x) => x.key === opensOn(state, run, p, dots));
      if (!after || d?.kind !== "run" || !d.path) continue;
      const path = d.path;
      void granted(d.runs[0]!.id).then(
        (g) => {
          if (stale) return;
          const before = fileUrl(g.files, path);
          (isFilm(p.picture) ? prefetch : preload)([before]);
          compared({ url: before, marks: d.marks }, { url: fileUrl(grant, after), marks: p.picture.marks ?? null }, isFilm(p.picture), true).catch(() => {});
        },
        () => {},
      );
    }
  });

  onMount(() => {
    // Y said as it is let go: held, it opens the choice of how far looks right reaches.
    let holding: ReturnType<typeof setTimeout> | undefined;
    // Keys press what a person could press, so they do only what the page offers them.
    const press = (selector: string) => document.querySelector<HTMLButtonElement>(selector)?.click();
    const keys = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (e.metaKey || e.ctrlKey || target?.closest("input, textarea, select, [contenteditable]")) return;
      if (e.altKey) return;
      // The board, while open, takes its own keys first (see Reach).
      if (e.key === "y") {
        if (!e.repeat && !holding) holding = setTimeout(() => ((holding = undefined), setMenu("float")), HOLD_MS);
      } else if (e.key === "j") step(1);
      else if (e.key === "k") step(-1);
      else if (/^[1-5]$/.test(e.key)) setMode(MODES[Number(e.key) - 1]![0]);
      else if (e.key === "u") press("[data-withdraw]:not(:disabled)");
      else if (e.key === "n") press("[data-whole]");
      // The first note offered from another variant, added here; pressed again, the next.
      else if (e.key === "a") press("[data-copy-note]");
      else if (e.key === "t") swap("theme");
      else if (e.key === "v") swap("viewport");
      else if (e.key === "?") setKeysShown((v) => !v);
      else if (e.key === "f") toggleFull();
      else if (e.key === "<") stepSpeed(-1);
      else if (e.key === ">") stepSpeed(1);
      else if (e.key === "+" || e.key === "=") press("[data-zoom=in]");
      else if (e.key === "-") press("[data-zoom=out]:not(:disabled)");
      else if (e.key === "0") press("[data-zoom=fit]");
      else if (e.key === " " && view().s > 1) setPanning(true);
      // Not zoomed, Space plays or pauses a film, as a player's does.
      else if (e.key === " " && document.querySelector("[data-stage] [data-play]")) {
        if (!e.repeat) press("[data-stage] [data-play]");
      }
      else if (e.key === "Escape" && keysShown()) setKeysShown(false);
      else if (e.key === "Escape" && full()) toggleFull();
      else return;
      e.preventDefault();
    };
    const released = (e: KeyboardEvent) => {
      if (e.key === " ") setPanning(false);
      // Let go before it opened the choice: this picture.
      if (e.key.toLowerCase() === "y" && holding) {
        clearTimeout(holding);
        holding = undefined;
        press('[data-say="looks_right"][aria-pressed="false"]:not(:disabled)');
      }
    };
    // Leaving the browser's full screen (its own Esc) leaves the page's too.
    const left = () => !document.fullscreenElement && full() && setFull(false);
    document.addEventListener("keydown", keys);
    document.addEventListener("keyup", released);
    document.addEventListener("fullscreenchange", left);
    onCleanup(() => {
      document.removeEventListener("keydown", keys);
      document.removeEventListener("keyup", released);
      document.removeEventListener("fullscreenchange", left);
    });
  });

  return (
    <section class="pictures" data-pictures aria-labelledby="pictures-title">
      <div class="pictures-head">
        <h2 id="pictures-title" class="visually-hidden">Pictures</h2>
        <nav class="counts" aria-label="Show only">
          <For each={counts(pictures())}>
            {(c) => (
              <button
                type="button"
                class={`count ${c.change}`}
                data-filter={c.change}
                aria-pressed={filter() === c.change}
                title={filter() === c.change ? "Show all" : `Show only ${WORDS[c.change].toLowerCase()}`}
                onClick={() => setFilter(filter() === c.change ? null : c.change)}
              >
                <b class="num">{c.n}</b>
                <span>{WORDS[c.change].toLowerCase()}</span>
              </button>
            )}
          </For>
        </nav>
        <Show when={options().length > 1}>
          <div class="seg group-by" role="group" aria-label="Group by">
            <For each={options()}>
              {(option) => (
                <button
                  type="button"
                  data-group={option}
                  aria-pressed={grouping() === option}
                  title={`Group by ${GROUP_WORDS[option].toLowerCase()}`}
                  onClick={() => setBy(option)}
                >
                  {GROUP_WORDS[option]}
                </button>
              )}
            </For>
          </div>
        </Show>
        {/* A check that failed on the base commit has nothing to compare with: said once, its
            reason (with the log on the runner) only on hover. */}
        <For each={[...new Map(pictures().filter((p) => p.look.no_before).map((p) => [p.check, p.look] as const)).entries()]}>
          {([check, look]) => (
            <p class="no-before" data-no-before={check} title={`No before: ${look.no_before}`}>
              Every picture{checks() > 1 ? ` of ${check}` : ""} is new: the base commit has none to compare with
            </p>
          )}
        </For>
        <Show when={localBy(props.run)}>
          {(who) => (
            <p class="unproven" data-unproven title="A local run is the agent's word: notes reach the agent now, verdicts wait for CI">
              {who()}'s local run, notes only
            </p>
          )}
        </Show>
      </div>
      <div class="navigator" data-by={grouping()}>
        <For each={groups()}>
          {(group) => {
            const list = () => (
              <ul class="sheet">
                <For each={screens().get(group.key) ?? []}>
                  {(screen) => (
                    <ScreenTile screen={screen} run={props.run} state={props.state} files={files()} current={current()} checks={checks()} onPick={pick} />
                  )}
                </For>
              </ul>
            );
            return (
              <Show
                when={group.folded}
                fallback={
                  <div class="group" data-group-key={group.key} data-group-label={group.label}>
                    {/* By change, the counts above already name each group. */}
                    <Show when={grouping() !== "change"}>
                      <h3 class={group.change ?? ""}>{group.label}</h3>
                    </Show>
                    {list()}
                  </div>
                }
              >
                <details
                  class="group"
                  data-group-key={group.key}
                  data-group-label={group.label}
                  open={unfolded().has(group.key)}
                  onToggle={(e) => {
                    const open = e.currentTarget.open;
                    setUnfolded((s) => {
                      const next = new Set(s);
                      if (open) next.add(group.key);
                      else next.delete(group.key);
                      return next;
                    });
                  }}
                >
                  <summary>
                    <h3 class={group.change ?? ""}>
                      {group.label} <span class="n num">{group.pictures.length}</span>
                    </h3>
                  </summary>
                  {list()}
                </details>
              </Show>
            );
          }}
        </For>
      </div>
      <div class="table" data-table data-full={full() ? "" : undefined} ref={(el) => (table = el)}>
      {/* A new stage per picture, and the same one while live events redraw its picture. */}
      <div
        class="stage-hold"
        ref={(el) => {
          stageBox = el;
          // Load events do not bubble; caught on the way down, a picture's load lets go of the height.
          el.addEventListener("load", loaded, true);
          el.addEventListener("loadeddata", loaded, true);
        }}
        style={hold() ? { "min-height": `${hold()}px` } : {}}
      >
        <Show when={currentKey()} keyed>
          {(_key) => (
            <Stage
              shown={current()!}
              files={files()}
              head={props.run.head}
              mode={mode()}
              onMode={setMode}
              checks={checks()}
              noting={noting}
              onTime={setTime}
              view={view()}
              onView={setView}
              panning={panning()}
              full={full()}
              onFull={toggleFull}
              against={against()}
              strip={
                <Show when={worthShowing(dots())}>
                  <Strip dots={dots()} picked={dotKey()} onPick={pickDot} />
                </Show>
              }
            />
          )}
        </Show>
      </div>
      <Show when={current()}>
        {(p) => (
          <ReviewBar
            shown={p()}
            run={props.run}
            state={props.state}
            viewer={props.viewer}
            repository={props.state.work.repository}
            notes={notes()}
            canNote={canNote()}
            sideBySide={mode() === "side"}
            onlyNow={!!against()}
            draft={composing() ? draft() : undefined}
            time={isFilm(p().picture) ? time() : undefined}
            position={order().findIndex((o) => o.key === p().key) + 1}
            of={order().length}
            onStep={step}
            onSaid={onward}
            onKeys={() => setKeysShown((v) => !v)}
            elsewhere={fromVariants()}
            onYourLook={!!against()?.said}
            onLook={(() => {
              const look = dots().find((d) => d.look);
              return look && look.key !== dotKey() ? () => pickDot(look.key) : undefined;
            })()}
            variants={screenOf()?.variants ?? []}
            viewports={screenOf()?.viewports ?? []}
            files={files()}
            menu={menu()}
            onMenu={setMenu}
            onFocus={pick}
            onDraft={(d) => {
              setDraft(d);
              setComposing(!!d);
            }}
          />
        )}
      </Show>
      <Show when={keysShown()}>
        <dl class="keys" data-keys aria-label="Keys">
          <For each={KEYS}>
            {([keys, what]) => (
              <div>
                <dt>
                  <For each={keys}>{(k) => <kbd>{k}</kbd>}</For>
                </dt>
                <dd>{what}</dd>
              </div>
            )}
          </For>
        </dl>
      </Show>
      </div>
    </section>
  );
}

/** Every key of the pictures, as the list `?` shows says them. */
const KEYS: [string[], string][] = [
  [["J", "K"], "Next and previous picture"],
  [["Y"], "Looks right, then on to the next that waits; a film's reaches its still"],
  [["Hold Y"], "The board of this screen's variants: 1 to 4 pick a shape, arrows and Space a tile, Y says it"],
  [["U"], "Take back looks right"],
  [["N"], "Note the whole picture; drag on it to mark a region"],
  [["A"], "Add a note of another variant here, in its place"],
  [["T"], "The other theme of this screen"],
  [["V"], "The other viewport of this screen"],
  [["1", "5"], "Side by side, slider, onion skin, difference, blink"],
  [["F"], "Full screen, and back"],
  [["<", ">"], "Play films slower and faster"],
  [["+", "−", "0"], "Zoom in, out, and the whole picture; pinch zooms where the pointer is"],
  [["Space"], "Play or pause a film; zoomed in, hold and drag to move the picture"],
  [["⌘", "Enter"], "Add the note"],
  [["Esc"], "Cancel the note"],
  [["?"], "Show or hide these keys"],
];

/** A picture's mark on its tile: looks right, or its open notes. */
function Said(props: { state: Snapshot; run: Run; shown: Shown }) {
  return (
    <Switch>
      <Match when={standingOf(props.state, props.run, props.shown) === "looks_right"}>
        <span class="right" aria-label="looks right" />
      </Match>
      <Match when={notesOn(props.state, props.shown.check, props.shown.picture.name, props.run).length}>
        {(n) => (
          <span class="noted" aria-label={`needs change, ${n()} open ${n() === 1 ? "note" : "notes"}`}>
            {n()}
          </span>
        )}
      </Match>
    </Switch>
  );
}

/** What a picture's own attributes say, for a button that picks it. */
function pictureData(state: Snapshot, run: Run, p: Shown) {
  return {
    "data-picture": p.key,
    "data-change": p.picture.change,
    "data-flaky": String(!!p.picture.flaky),
    "data-verdict": standingOf(state, run, p),
    "data-again": changedSince(state, run, p) ? "" : undefined,
  };
}

/** One screen in the sheet: a picture of it, and, when it has variants, one small button per
 * theme under each viewport. The tile shows the variant on the stage, else its first. */
function ScreenTile(props: {
  screen: Screen;
  run: Run;
  state: Snapshot;
  files: string;
  current: Shown | undefined;
  checks: number;
  onPick: (key: string) => void;
}) {
  const single = () => props.screen.variants.length === 1;
  const here = () => props.screen.variants.find((p) => p.key === props.current?.key);
  const shown = () => here() ?? props.screen.variants[0]!;
  const allRight = () => props.screen.variants.every((p) => standingOf(props.state, props.run, p) === "looks_right");
  const again = () => props.screen.variants.some((p) => changedSince(props.state, props.run, p));
  const word = (p: Shown) => `${WORDS[p.picture.change].toLowerCase()}${p.picture.flaky ? ", flaky" : ""}${props.checks > 1 ? `, ${p.check}` : ""}`;
  return (
    <li class="tile" data-screen={props.screen.key} aria-current={here() ? "true" : undefined}>
      <button
        type="button"
        class="thumb"
        {...(single() ? pictureData(props.state, props.run, shown()) : {})}
        aria-current={here() ? "true" : undefined}
        title={single() ? `${title(shown().parts)}: ${word(shown())}` : `${props.screen.label}: ${props.screen.variants.length} variants`}
        onClick={() => props.onPick(shown().key)}
      >
        <span class="shot">
          <Thumb shown={shown()} files={props.files} />
          <Show when={again()}>
            <span class="again" data-again-mark title="It changed since a person said it looks right, or since its note">
              Look again
            </span>
          </Show>
          <Show when={!single() && allRight()} fallback={<Said state={props.state} run={props.run} shown={shown()} />}>
            <span class="right" aria-label="every variant looks right" />
          </Show>
          <Show when={isFilm(shown().picture)}>
            <span class="film-mark" aria-label="film" />
          </Show>
        </span>
        <span class="name">
          <span class={`dot ${shown().picture.change}`} aria-hidden="true" />
          {single() ? title(shown().parts) : props.screen.label}
        </span>
      </button>
      <Show when={!single()}>
        <span class="variants">
          <For each={props.screen.viewports}>
            {(v) => (
              <span class="viewport">
                <Show when={v.viewport}>
                  <span class="vp">{v.viewport}</span>
                </Show>
                <span class="swatches">
                  <For each={v.variants}>
                    {(p) => (
                      <button
                        type="button"
                        class={`variant theme-${p.parts.theme || "none"} ${isFilm(p.picture) ? "of-film" : ""} ${p.parts.theme || p.parts.viewport ? "" : "named"}`}
                        {...pictureData(props.state, props.run, p)}
                        aria-current={props.current?.key === p.key ? "true" : undefined}
                        aria-label={`${title(p.parts)}: ${word(p)}`}
                        title={`${[p.parts.viewport, p.parts.theme].filter(Boolean).join(", ")}: ${word(p)}`}
                        onClick={() => props.onPick(p.key)}
                      >
                        {p.parts.theme || p.parts.viewport ? "" : p.picture.name}
                      </button>
                    )}
                  </For>
                </span>
              </span>
            )}
          </For>
        </span>
      </Show>
    </li>
  );
}

/** Under the picture on the stage: what this person can say about it and where it stands, the
 * way to the next picture, the notes on it, and the note being written. */
function ReviewBar(props: {
  shown: Shown;
  run: Run;
  state: Snapshot;
  viewer: Looker | undefined;
  repository: string;
  notes: Noted[];
  canNote: boolean;
  sideBySide: boolean;
  /** An earlier run is on the left, so notes go on now's side only. */
  onlyNow: boolean;
  draft: Draft | undefined;
  time: number | undefined;
  position: number;
  of: number;
  onStep: (by: number) => void;
  /** Its verdict was taken: on to the next picture that waits. */
  onSaid: () => void;
  onKeys: () => void;
  /** Open notes on the screen's other variants. */
  elsewhere: Elsewhere[];
  onDraft: (draft: Draft | undefined) => void;
  /** Every variant of the screen, for a looks right that reaches more than this picture. */
  variants: Shown[];
  /** The same variants per viewport, as the board of how far looks right reaches shows them. */
  viewports: Screen["viewports"];
  files: string;
  /** Compares now with the run of the last looks right, when the strip has it and it is not
   * shown already. */
  onLook: (() => void) | undefined;
  /** Whether the stage compares with your look, whose line above the picture says who said it
   * looks right, so it is not said again here. */
  onYourLook: boolean;
  /** Where the choice of how far a looks right reaches is open: beside its arrow, or floating
   * on the screen for a held Y. */
  menu: Menu;
  onMenu: (open: Menu) => void;
  /** A tile focused on the board: its picture is shown behind it. */
  onFocus: (key: string) => void;
}) {
  const [error, setError] = createSignal("");
  const [words, setWords] = createSignal("");
  // The note whose words the draft took, so the new note stays linked with it.
  const [from, setFrom] = createSignal<string>();
  // The note being edited, and its words so far.
  const [editing, setEditing] = createSignal<{ id: string; words: string }>();
  // Whether an edit takes the linked notes along: almost always, so it starts checked.
  const [alsoLinked, setAlsoLinked] = createSignal(true);
  const verdict = createMemo(() => verdictOf(props.state, props.run, props.shown));
  // Why this picture gets no verdict, in a few words; the reason in full is its title.
  const never = (): [string, string] | undefined => {
    if (localBy(props.run)) return ["Verdicts on CI", "A local run is unproven, so its pictures get a verdict on CI."];
    if (props.shown.picture.flaky) return ["Flaky", "Flaky pictures change from run to run, so they never wait for a verdict."];
    if (!toLookAt(props.shown)) return ["Unchanged", "Unchanged pictures need no verdict."];
    return undefined;
  };
  const standing = createMemo(() => standingOf(props.state, props.run, props.shown));
  const state = () => (never() ? "never" : standing());
  const since = createMemo(() => changedSince(props.state, props.run, props.shown));
  // The numbers of the notes made before it changed, as the list numbers them.
  const sinceNotes = () => props.notes.filter((n) => since()?.notes.includes(n.note.id)).map((n) => n.n);
  // With an earlier run on the left, a note goes on now's side only: the before it would also
  // mark is not shown.
  const both = () => !props.onlyNow && props.shown.picture.before != null && props.shown.picture.after != null;
  // Who says it: the page shows what they say at once, and the host settles it behind.
  const by = () => signedIn(props.viewer)?.login ?? "";
  // What a plain looks right says looks right: this picture, and a film's still that does not
  // look right yet.
  const reach = (scope: "this") =>
    covered(props.shown, props.variants, scope).filter((q, k) => k === 0 || standingOf(props.state, props.run, q) !== "looks_right");
  // On to the next picture at once; a refusal is said on whichever picture shows then.
  const send = async (list: Shown[] = reach("this")) => {
    setError("");
    props.onMenu(false);
    const named = title(props.shown.parts);
    const said = list.map((q) => say(props.state, props.run, q, by()));
    props.onSaid();
    const refused = (await Promise.all(said)).filter((r): r is string => !!r);
    if (refused.length) setError(list.length > 1 ? `Looks right on ${refused.length} of ${list.length}: ${refused[0]}` : `Looks right on ${named}: ${refused[0]}`);
  };
  // A press outside the choice closes it.
  onMount(() => {
    // Moving to another picture keeps it open, its picks kept, for marking quickly.
    const away = (e: PointerEvent) =>
      props.menu && !(e.target as HTMLElement | null)?.closest("[data-say-more], [data-say-menu], [data-picture], [data-step]") && props.onMenu(false);
    document.addEventListener("pointerdown", away);
    onCleanup(() => document.removeEventListener("pointerdown", away));
  });
  const whole = () => {
    const side = props.shown.picture.after ? "after" : "before";
    props.onDraft({ side, region: { x: 0, y: 0, w: 1, h: 1 } });
    queueMicrotask(() => document.querySelector<HTMLElement>("textarea[data-words]")?.focus());
  };
  const add = async (e: Event) => {
    e.preventDefault();
    const d = props.draft;
    const text = words().trim();
    if (!d || !text) return;
    const link = from();
    const said = makeNote(
      props.run,
      {
        check: props.shown.check,
        picture: props.shown.picture.name,
        side: sidesOf(d, !!d.both),
        region: d.region,
        ...(props.time != null ? { time: hundredths(props.time) } : {}),
        words: text,
        ...(link ? { link } : {}),
      },
      by(),
    );
    setError("");
    setWords("");
    setFrom(undefined);
    props.onDraft(undefined);
    const refused = await said;
    // Refused, the note comes back to write again, as it was.
    if (refused) {
      setError(refused);
      setWords(text);
      setFrom(link);
      props.onDraft(d);
    }
  };
  // Saves the words being edited, on that note alone or with `linked` on its linked notes too.
  const save = async (linked: boolean) => {
    const e = editing();
    const text = e?.words.trim();
    const note = props.notes.find((n) => n.note.id === e?.id)?.note;
    if (!e || !text || !note) return;
    setError("");
    setEditing(undefined);
    const refused = await editNote(props.state, note, text, linked);
    if (refused) {
      setError(refused);
      setEditing(e);
    }
  };
  const undo = async () => {
    setError("");
    setError((await withdraw(props.run, props.shown)) ?? "");
  };
  // A note of another variant, made here in its place and words.
  const copy = async (from: Elsewhere) => {
    setError("");
    const refused = await makeNote(
      props.run,
      {
        check: props.shown.check,
        picture: props.shown.picture.name,
        side: from.side,
        region: from.note.region,
        ...(from.note.time != null ? { time: from.note.time } : {}),
        words: from.note.words,
        link: from.note.id,
      },
      by(),
    );
    setError(refused ?? "");
  };
  // Its words in the note being written; with none, a note of the whole picture to drag on.
  const useWords = (from: Elsewhere) => {
    if (!props.draft) whole();
    setWords(from.note.words);
    setFrom(from.note.id);
    queueMicrotask(() => document.querySelector<HTMLElement>("textarea[data-words]")?.focus());
  };
  const resolve = async (note: Note) => {
    setError("");
    setError((await resolveNote(props.state, note, by())) ?? "");
  };
  const noteHint = () =>
    isText(props.shown.picture)
      ? "Note what should change in this text"
      : props.sideBySide
      ? `Drag on ${both() ? "either picture" : "the picture"} to mark what should change, or note the whole picture`
      : "Side by side, drag on a picture to mark what should change; or note the whole picture";
  return (
    <div class="verdict" data-said={state()} aria-live="polite">
      <div class="decide">
        <Switch>
          <Match when={never()}>{(why) => <p class="hint" title={why()[1]}>{why()[0]}</p>}</Match>
          <Match when={onlyViews(props.viewer)}>
            <p class="hint" title="Verdicts are given on the work's own page">
              Only views
            </p>
          </Match>
          <Match when={canSay(props.viewer)}>
            <button
              type="button"
              class="btn ok"
              data-say="looks_right"
              aria-pressed={!!verdict()}
              title={
                verdict()
                  ? "Take back looks right"
                  : props.notes.length
                    ? "Looks right resolves its notes, and holds as long as it stays the same"
                    : "Looks right holds as long as it stays the same"
              }
              // Pressed again, it takes the verdict back, as Undo does.
              onClick={() => void (verdict() ? undo() : send())}
            >
              <Icon name="check" size={16} />
              Looks right
            </button>
            <span class="say-more">
              <button
                type="button"
                class="btn ok more"
                data-say-more
                aria-haspopup="dialog"
                aria-expanded={!!props.menu}
                aria-label="How far looks right reaches"
                title="How far looks right reaches (hold Y)"
                onClick={() => props.onMenu(props.menu ? false : "here")}
              >
                <Icon name="chevron" size={14} />
              </button>
              <Reach
                shown={props.shown}
                variants={props.variants}
                viewports={props.viewports}
                state={props.state}
                run={props.run}
                files={props.files}
                menu={props.menu}
                onMenu={props.onMenu}
                onFocus={props.onFocus}
                onSay={(list: Shown[]) => void send(list)}
              />
            </span>
          </Match>
          <Match when={signedIn(props.viewer)}>
            <p class="hint" data-cannot title={`Giving a verdict needs write access to ${props.repository}`}>
              Needs write access
            </p>
          </Match>
          <Match when={maySignIn(props.viewer)}>
            <a class="plain" href={signIn()} data-sign-in title="Sign in to say whether it looks right">
              Sign in to review
            </a>
          </Match>
        </Switch>
        <Show when={props.canNote && !props.draft}>
          <button type="button" class="btn" data-whole title={noteHint()} onClick={whole}>
            Note
          </button>
        </Show>
        <Show when={standing() === "needs_change"}>
          <p class="said bad" title={`${props.notes.length} open ${props.notes.length === 1 ? "note" : "notes"}`}>
            <span class="needs" aria-hidden="true" />
            <b>Needs change</b>
          </p>
        </Show>
        <Show when={standing() === "looks_right" && verdict()}>
          {(v) => (
            <p class="said ok" title={[ago(Date.now() / 1000 - v().at), carried(v(), props.run)].filter(Boolean).join(", ")}>
              <span class="right" aria-hidden="true" />
              <span>
                by <b>{v().by}</b>
              </span>
              <Show when={canSay(props.viewer) && !never()}>
                <button type="button" class="plain" data-withdraw title="Take back looks right (u)" onClick={() => void undo()}>
                  Undo
                </button>
              </Show>
            </p>
          )}
        </Show>
        <Show when={since() && (sinceNotes().length || !props.onYourLook) ? since() : undefined}>
          {(s) => (
            <p class="said again" data-again title="The verdict and the notes stay as they were: say looks right again, or resolve the notes">
              <Show
                when={sinceNotes().length}
                fallback={
                  <span>
                    Changed since <b>{s().verdict?.by}</b> said it looks right at {short(s().verdict?.head ?? "")}: look again
                    <Show when={props.onLook}>
                      {(go) => (
                        <>
                          {", "}
                          <button type="button" class="plain" data-compare-look title="Compare now with what it showed when it looked right" onClick={() => go()()}>
                            compare with that look
                          </button>
                        </>
                      )}
                    </Show>
                  </span>
                }
              >
                <span>
                  Changed since {sinceNotes().length === 1 ? "note" : "notes"} <b>{sinceNotes().join(", ")}</b>: check {sinceNotes().length === 1 ? "it" : "them"}, then resolve or keep
                </span>
              </Show>
            </p>
          )}
        </Show>
        <span class="step">
          <button type="button" class="ghost" aria-label="Previous picture" title="Previous (k)" data-step onClick={() => props.onStep(-1)} disabled={props.position <= 1}>
            ‹
          </button>
          <span class="num">
            {props.position} of {props.of}
          </span>
          <kbd title="Previous picture">K</kbd>
          <kbd title="Next picture">J</kbd>
          <button type="button" class="ghost keys-toggle" title="Keys (?)" aria-label="Show the keys" onClick={() => props.onKeys()}>
            <kbd>?</kbd>
          </button>
          <button type="button" class="ghost" aria-label="Next picture" title="Next (j)" data-step onClick={() => props.onStep(1)} disabled={props.position >= props.of}>
            ›
          </button>
        </span>
      </div>
      <Show when={props.notes.length}>
        <ol class="note-list" data-notes aria-label="Notes on this picture">
          <For each={props.notes}>
            {(n) => (
              <li data-note={n.note.id} data-n={n.n} data-old={String(n.old)} class={n.old ? "old" : ""}>
                <span class="n" aria-hidden="true">
                  {n.n}
                </span>
                <Show
                  when={editing()?.id === n.note.id}
                  fallback={
                <p class="words">
                  {n.note.words}
                  <span class="who" title={ago(Date.now() / 1000 - n.note.at)}>
                    {n.note.by}, {n.note.side === "both" ? "both sides" : n.note.side}
                    <Show when={n.note.time != null}>, at {n.note.time!.toFixed(2)} s</Show>
                    <Show when={fromWhere(n.note, props.run)}>{(where) => <>, {where()}</>}</Show>
                  </span>
                  <Show when={since()?.notes.includes(n.note.id)}>
                    <span class="changed" data-changed-since>
                      changed since
                    </span>
                  </Show>
                </p>
                  }
                >
                  {/* Its words, to change; linked notes take them too when the person says so. */}
                  <form class="edit-note" data-edit-note onSubmit={(e) => (e.preventDefault(), void save(alsoLinked() && linkedWith(props.state, n.note).length > 0))}>
                    <textarea
                      data-edit-words
                      rows="3"
                      ref={grows}
                      value={editing()?.words ?? ""}
                      aria-label="The note's words"
                      onInput={(e) => setEditing({ id: n.note.id, words: e.currentTarget.value })}
                      onKeyDown={(e) => {
                        if (e.key === "Escape") setEditing(undefined);
                        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) e.currentTarget.form?.requestSubmit();
                      }}
                    />
                    <span class="act">
                      <button type="submit" class="btn ok" data-save-note>
                        Save
                      </button>
                      <Show when={linkedWith(props.state, n.note).length}>
                        {(others) => (
                          <label class="also" title="The same words on the linked notes of the screen's other variants">
                            <input type="checkbox" data-save-linked checked={alsoLinked()} onChange={(e) => setAlsoLinked(e.currentTarget.checked)} />
                            Also on {others() === 1 ? "the linked note" : `the ${others()} linked notes`}
                          </label>
                        )}
                      </Show>
                      <button type="button" class="plain" onClick={() => setEditing(undefined)}>
                        Cancel
                      </button>
                    </span>
                  </form>
                </Show>
                <Show when={props.canNote && editing()?.id !== n.note.id}>
                  <span class="note-acts">
                    <button
                      type="button"
                      class="plain"
                      data-edit
                      disabled={pending(n.note)}
                      onClick={() => {
                        setEditing({ id: n.note.id, words: n.note.words });
                        setAlsoLinked(true);
                        queueMicrotask(() => document.querySelector<HTMLElement>("textarea[data-edit-words]")?.focus());
                      }}
                    >
                      Edit
                    </button>
                    <button type="button" class="plain resolve" data-resolve disabled={pending(n.note)} onClick={() => void resolve(n.note)}>
                      Resolve
                    </button>
                  </span>
                </Show>
              </li>
            )}
          </For>
        </ol>
      </Show>
      <Show when={props.canNote && props.elsewhere.length}>
        <div class="elsewhere" data-elsewhere>
          <p class="label">On other variants</p>
          <ul>
            <For each={props.elsewhere}>
              {(from, k) => (
                <li data-from={from.note.id}>
                  <p class="words">
                    {from.note.words}
                    <span class="who">{from.where}</span>
                  </p>
                  <span class="act">
                    <Show when={from.place}>
                      <button
                        type="button"
                        class="btn"
                        data-copy-note
                        title="The same words on the same place of this picture (a)"
                        onClick={() => void copy(from)}
                      >
                        Add here
                        <Show when={k() === props.elsewhere.findIndex((o) => o.place)}>
                          <kbd>A</kbd>
                        </Show>
                      </button>
                    </Show>
                    <button type="button" class="plain" data-use-words title="These words in a note on this picture; drag to mark where" onClick={() => useWords(from)}>
                      Use words
                    </button>
                  </span>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>
      <Show when={props.canNote && props.draft}>
        {(d) => (
          <form class="compose" data-compose onSubmit={(e) => void add(e)}>
            <label class="words-field">
              <span class="label">
                Note {props.notes.length + 1}
                <Show when={props.time != null}> at {props.time!.toFixed(2)} s</Show>
              </span>
              <textarea
                data-words
                rows="3"
                ref={grows}
                maxLength={2000}
                placeholder="What should change? Drag on the picture to point at it."
                value={words()}
                onInput={(e) => setWords(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") props.onDraft(undefined);
                  else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void add(e);
                }}
              />
            </label>
            <div class="act">
              <button type="submit" class="btn" data-add-note disabled={!words().trim()}>
                Add note
              </button>
              <Show when={both()}>
                <label class="check">
                  <input
                    type="checkbox"
                    data-both
                    checked={!!d().both}
                    onChange={(e) => props.onDraft({ ...d(), both: e.currentTarget.checked })}
                  />
                  Both sides
                </label>
              </Show>
              <button type="button" class="plain" data-cancel-note onClick={() => props.onDraft(undefined)}>
                Cancel
              </button>
            </div>
          </form>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </div>
  );
}
