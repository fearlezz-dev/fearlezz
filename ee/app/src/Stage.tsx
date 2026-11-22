// One picture on the light table: its before and after, the five ways to compare them, and for
// a film both films on one clock with lanes of where they differ. A text shows as its diff. Side by side, the notes on the
// picture show as numbered regions, and a person who may make notes drags on either picture to
// draw one.

import { type JSX, For, Match, Show, Switch, createContext, createEffect, createMemo, createResource, createSignal, onCleanup, splitProps, untrack, useContext } from "solid-js";
import { DiffLines } from "./DiffLines";
import { Icon } from "./icons";
import { fileUrl } from "./grant";
import { type Draft, type Shown as Noted, meant, onSide, regionBetween } from "./notes";
import { type Box, type Shown, WORDS, isFilm, isText, lanes, marksAt, plainSummary, stillMarks, title } from "./pictures";
import type { Comparison } from "./proof";
import { ready } from "./preload";
import { short } from "./short";
import { FIT, type View, panned, stepped, transform, zoomedAt } from "./zoom";

/** The ways to compare: key, name, and the short name a phone shows. */
export const MODES = [
  ["side", "Side by side", "Side"],
  ["slider", "Slider", "Slider"],
  ["onion", "Onion skin", "Onion"],
  ["diff", "Difference", "Diff"],
  ["blink", "Blink", "Blink"],
] as const;

export type Mode = (typeof MODES)[number][0];

type Side = "before" | "after";

/** An earlier run's picture on the left side in place of the base: its file, what it is
 * called, whether it is the person's last look (drawn in the Look again colour), and what
 * comparing it with now in the page found, once it has (or why it could not). */
export type Against = {
  url: string;
  word: string;
  label: string;
  look: boolean;
  comparison: Comparison | undefined;
  error: string | undefined;
};

/** What comparing an earlier run's picture with now came to, in a few words. */
export function pairSummary(c: Comparison, word: string): string {
  const than = word.toLowerCase();
  switch (c.verdict) {
    case "same":
      return `Looks the same as ${than}.`;
    case "timing":
      return `The same pictures as ${than}, with other timing.`;
    case "changed":
      if (c.size_before[0] !== c.size_after[0] || c.size_before[1] !== c.size_after[1]) return `Another size than ${than}.`;
      return c.still
        ? `Changed since ${than} in ${c.moments.length} ${c.moments.length === 1 ? "place" : "places"}.`
        : `Changed since ${than} at ${c.moments.map((m) => `${m.after[0].toFixed(2)} s`).join(", ")}.`;
  }
}

/** What the stage needs to show notes and let a person draw one. */
export type Noting = {
  /** The open notes on the picture, numbered. */
  notes: Noted[];
  /** The region being drawn, or drawn and waiting for words. */
  draft: Draft | undefined;
  /** Whether the person looking may make notes. */
  can: boolean;
  /** The one side notes show and are drawn on, when the other shows no picture of this run's
   * proof (an earlier run's). */
  only?: Side | undefined;
  /** A region was drawn (`done` once the pointer lifted), or the draft dropped. */
  onDraft: (draft: Draft | undefined, done: boolean) => void;
};

/** The notes on one side of a picture, and the layer a person draws a new one on. On a film
 * (`clock`), a click plays or pauses it, and a drag pauses it first, so its note marks the
 * frame it was drawn on. */
function NoteLayer(props: { side: Side; noting: Noting; clock?: Clock | undefined }) {
  let layer: HTMLDivElement | undefined;
  let start: [number, number] | undefined;
  // Whether the film played when the pointer went down: a click then leaves it paused.
  let played = false;
  const at = (e: PointerEvent): [number, number] => {
    const r = layer!.getBoundingClientRect();
    return [(e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height];
  };
  const pct = (v: number) => `${v * 100}%`;
  const here = () => !props.noting.only || props.noting.only === props.side;
  const can = () => props.noting.can && here();
  const regions = () => (here() ? props.noting.notes.filter((n) => onSide(n.note, props.side)) : []);
  const draft = () => {
    const d = props.noting.draft;
    return d && here() && (d.side === props.side || d.both) ? d : undefined;
  };
  return (
    <div
      ref={layer}
      class={`notes-layer ${can() ? "can" : ""} ${props.clock ? "plays" : ""}`}
      data-draw={props.side}
      onPointerDown={(e) => {
        if ((!can() && !props.clock) || e.button !== 0) return;
        e.preventDefault();
        layer!.setPointerCapture(e.pointerId);
        start = at(e);
        played = !!props.clock?.playing();
        if (played) props.clock!.toggle();
      }}
      onPointerMove={(e) => {
        if (!start || !can()) return;
        props.noting.onDraft({ side: props.side, region: regionBetween(start, at(e)) }, false);
      }}
      onPointerUp={(e) => {
        if (!start) return;
        const region = regionBetween(start, at(e));
        start = undefined;
        const drawn = can() && meant(region);
        if (can()) props.noting.onDraft(drawn ? { side: props.side, region } : undefined, drawn);
        // A click, not a drag: a paused film plays.
        if (!drawn && props.clock && !played) props.clock.toggle();
      }}
      onPointerCancel={() => {
        start = undefined;
        props.noting.onDraft(undefined, false);
      }}
    >
      <For each={regions()}>
        {(n) => (
          <div
            class={`region ${n.old ? "old" : ""}`}
            data-region
            data-note={n.note.id}
            data-n={n.n}
            data-old={String(n.old)}
            data-says={n.note.words}
            style={{ left: pct(n.note.region.x), top: pct(n.note.region.y), width: pct(n.note.region.w), height: pct(n.note.region.h) }}
          >
            <i>{n.n}</i>
          </div>
        )}
      </For>
      <Show when={draft()}>
        {(d) => (
          <div
            class="region draft"
            data-region
            data-draft
            style={{ left: pct(d().region.x), top: pct(d().region.y), width: pct(d().region.w), height: pct(d().region.h) }}
          >
            <i>{props.noting.notes.length + 1}</i>
          </div>
        )}
      </Show>
    </div>
  );
}

/** How fast films play, against real time: slower to see a change happen, faster to get
 * through a long film. */
export const SPEEDS = [0.25, 0.5, 1, 2, 4] as const;

// One speed for every film of the page, so it holds while a person steps from film to film.
const [speed, setSpeed] = createSignal<number>(1);

/** The next speed slower (`by` -1) or faster (1), staying at the slowest and fastest. */
export function stepSpeed(by: -1 | 1) {
  const at = SPEEDS.indexOf(speed() as (typeof SPEEDS)[number]);
  setSpeed(SPEEDS[Math.max(0, Math.min(SPEEDS.length - 1, at + by))]!);
}

/** The films' one clock: where both are, whether they play, and the videos it moves. Its length
 * grows to the longest film it moves, for a film alone that no comparison measured. */
type Clock = {
  t: () => number;
  playing: () => boolean;
  total: () => number;
  /** Whether every film it moves can play through. */
  ready: () => boolean;
  add: (video: HTMLVideoElement) => void;
  seek: (t: number) => void;
  toggle: () => void;
};

/** A clock for films at least `length` seconds long, which plays once, on its own, as soon as
 * every film it moves can play through while `autoplay` holds, so both sides start together. */
function createClock(length: () => number, autoplay: () => boolean = () => false): Clock {
  let starts = true;
  const [loaded, setLoaded] = createSignal(new Set<HTMLVideoElement>());
  const [longest, setLongest] = createSignal(0);
  const total = () => Math.max(length(), longest(), 0.01);
  const [t, setT] = createSignal(0);
  const [playing, setPlaying] = createSignal(false);
  const videos = new Set<HTMLVideoElement>();
  let frame = 0;
  // A film shorter than the clock rests on its last frame.
  const end = (v: HTMLVideoElement) => (Number.isFinite(v.duration) ? v.duration : total()) - 0.01;
  const place = (v: HTMLVideoElement, at: number) => {
    v.currentTime = Math.max(0, Math.min(at, end(v)));
  };
  const stop = () => {
    setPlaying(false);
    cancelAnimationFrame(frame);
    for (const v of videos) v.pause();
  };
  const seek = (at: number) => {
    stop();
    const clamped = Math.max(0, Math.min(total(), at));
    setT(clamped);
    for (const v of videos) place(v, clamped);
  };
  const play = () => {
    if (t() >= total() - 0.05) seek(0);
    setPlaying(true);
    // Where the clock was and when, at the speed it has played since: a new speed starts here.
    let from = t();
    let since = performance.now();
    let rate = speed();
    const tick = (now: number) => {
      if (speed() !== rate) {
        from += ((now - since) / 1000) * rate;
        since = now;
        rate = speed();
      }
      const at = from + ((now - since) / 1000) * rate;
      if (at >= total()) {
        setT(total());
        stop();
        return;
      }
      setT(at);
      for (const v of videos) {
        if (at < end(v)) {
          if (v.playbackRate !== rate) v.playbackRate = rate;
          if (v.paused) void v.play().catch(() => {});
          // Films drift apart as they play; one that falls behind the clock is put back.
          if (Math.abs(v.currentTime - at) > 0.15 * Math.max(1, rate)) v.currentTime = at;
        } else if (!v.paused) v.pause();
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
  };
  const [count, setCount] = createSignal(0);
  const ready = () => count() > 0 && loaded().size >= count();
  createEffect(() => {
    if (starts && ready() && autoplay()) {
      starts = false;
      untrack(play);
    }
  });
  onCleanup(stop);
  return {
    t,
    playing,
    total,
    ready,
    add: (video) => {
      videos.add(video);
      setCount(videos.size);
      video.addEventListener("loadedmetadata", () => {
        if (Number.isFinite(video.duration)) setLongest((was) => Math.max(was, video.duration));
        place(video, t());
      });
      video.addEventListener("canplaythrough", () => setLoaded((was) => new Set(was).add(video)));
      onCleanup(() => {
        videos.delete(video);
        setCount(videos.size);
        setLoaded((was) => {
          const now = new Set(was);
          now.delete(video);
          return now;
        });
      });
    },
    seek,
    toggle: () => (playing() ? stop() : play()),
  };
}

/** A still that keeps showing the picture before it until its own is fetched and decoded, so a
 * step never blanks the stage or changes its height twice; one fetched ahead swaps in at once. */
function Still(props: { src: string; side: Side; label: string }) {
  const [shown, setShown] = createSignal(props.src);
  createEffect(() => {
    const src = props.src;
    if (src === untrack(shown)) return;
    void ready(src).then(() => {
      if (props.src === src) setShown(src);
    });
  });
  return <img src={shown()} alt={props.label} data-side={props.side} draggable={false} decoding="async" />;
}

/** One side of a picture: an image, or a video its clock moves. */
function Media(props: { src: string; side: Side; clock: Clock | undefined; label: string }) {
  return (
    <Show when={props.clock} fallback={<Still src={props.src} side={props.side} label={props.label} />}>
      {(clock) => (
        <video
          ref={(v) => clock().add(v)}
          src={props.src}
          data-side={props.side}
          aria-label={props.label}
          muted
          playsinline
          preload="auto"
        />
      )}
    </Show>
  );
}

/** Outlines of where the two differ, on a frame as large as the picture shows. */
function Marks(props: { boxes: Box[]; size: [number, number] | undefined }) {
  const pct = (v: number, of: number) => `${(v / of) * 100}%`;
  return (
    <Show when={props.size}>
      {(size) => (
        <div class="marks" aria-hidden="true">
          <For each={props.boxes}>
            {(b) => (
              <div
                class="mark"
                style={{
                  left: pct(b[0], size()[0]),
                  top: pct(b[1], size()[1]),
                  width: pct(b[2] - b[0], size()[0]),
                  height: pct(b[3] - b[1], size()[1]),
                }}
              />
            )}
          </For>
        </div>
      )}
    </Show>
  );
}

/** The zoom every frame of the stage shares, so both sides always show the same place. */
type Zooming = { view: () => View; onView: (view: View) => void; panning: () => boolean };
const Zoom = createContext<Zooming>({ view: () => FIT, onView: () => {}, panning: () => false });

/** Zooms with a pinch (a wheel with Control or Command held) around the pointer, and moves a
 * zoomed picture with the wheel, or by dragging with Space held, the middle button, or anywhere
 * a drag means nothing else. */
function zoomable(frame: HTMLDivElement, zoom: Zooming) {
  const at = (e: { clientX: number; clientY: number }): [number, number] => {
    const r = frame.getBoundingClientRect();
    return [(e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height];
  };
  frame.addEventListener(
    "wheel",
    (e) => {
      const view = zoom.view();
      if (e.ctrlKey || e.metaKey) zoom.onView(zoomedAt(view, Math.exp(-e.deltaY * 0.005), at(e)));
      else if (view.s > 1) {
        const r = frame.getBoundingClientRect();
        zoom.onView(panned(view, [-e.deltaX / r.width, -e.deltaY / r.height]));
      } else return;
      e.preventDefault();
    },
    { passive: false },
  );
  let from: [number, number] | undefined;
  frame.addEventListener(
    "pointerdown",
    (e) => {
      const claimed = (e.target as Element).closest(".notes-layer.can, .slide");
      if (zoom.view().s <= 1 || !(zoom.panning() || e.button === 1 || (e.button === 0 && !claimed))) return;
      e.stopPropagation();
      e.preventDefault();
      frame.setPointerCapture(e.pointerId);
      from = at(e);
    },
    true,
  );
  frame.addEventListener("pointermove", (e) => {
    if (!from) return;
    const now = at(e);
    zoom.onView(panned(zoom.view(), [now[0] - from[0], now[1] - from[1]]));
    from = now;
  });
  const end = () => (from = undefined);
  frame.addEventListener("pointerup", end);
  frame.addEventListener("pointercancel", end);
}

/** A frame as large as its picture shows, so outlines sit on the picture and not on its mat.
 * What it holds zooms with the stage's one view. */
function Frame(
  props: { size: [number, number] | undefined; class?: string; ref?: (el: HTMLDivElement) => void; children: JSX.Element } & Omit<
    JSX.HTMLAttributes<HTMLDivElement>,
    "class" | "style" | "children" | "ref"
  >,
) {
  const [own, rest] = splitProps(props, ["size", "class", "children", "ref"]);
  const zoom = useContext(Zoom);
  // The size its picture turned out to have, when no comparison said: without it the frame
  // would be wider than a picture its height limit letterboxes, and notes, kept as fractions of
  // the frame, would land elsewhere at another window height.
  const [learned, setLearned] = createSignal<[number, number]>();
  const learn = (e: Event) => {
    const media = e.target;
    const size: [number, number] | undefined =
      media instanceof HTMLImageElement
        ? [media.naturalWidth, media.naturalHeight]
        : media instanceof HTMLVideoElement
        ? [media.videoWidth, media.videoHeight]
        : undefined;
    if (size && size[0] > 0 && size[1] > 0) setLearned(size);
  };
  const style = () => {
    const size = own.size ?? learned();
    return size ? { "--ratio": `${size[0]} / ${size[1]}`, "--w": `${size[0]}`, "--h": `${size[1]}` } : {};
  };
  return (
    <div
      {...rest}
      ref={(el) => {
        zoomable(el, zoom);
        el.addEventListener("load", learn, true);
        el.addEventListener("loadedmetadata", learn, true);
        own.ref?.(el);
      }}
      class={`frame ${own.size ?? learned() ? "sized" : ""} ${zoom.view().s > 1 ? "zoomed" : ""} ${own.class ?? ""}`}
      style={style()}
    >
      <div
        class={`zoom-layer ${zoom.view().s >= 3 ? "sharp" : ""}`}
        data-zoom-layer
        style={{ transform: transform(zoom.view()), "--z": `${zoom.view().s}` }}
      >
        {own.children}
      </div>
    </div>
  );
}

/** The ways to compare a before with its after, each a different stage. */
function Compare(props: {
  mode: Mode;
  before: string;
  after: string;
  size: [number, number] | undefined;
  boxes: Box[];
  clock: Clock | undefined;
  beforeLabel: string;
  afterLabel: string;
  beforeWord: string;
  afterWord: string;
  noting: Noting;
}) {
  const media = (side: Side) => (
    <Media
      src={side === "before" ? props.before : props.after}
      side={side}
      clock={props.clock}
      label={side === "before" ? props.beforeLabel : props.afterLabel}
    />
  );
  const marks = () => <Marks boxes={props.boxes} size={props.size} />;
  return (
    <Switch>
      <Match when={props.mode === "side"}>
        <div class="layers side">
          <figure>
            <figcaption title={props.beforeLabel} data-caption="before">{props.beforeWord}</figcaption>
            <Frame size={props.size}>
              {media("before")}
              {marks()}
              <NoteLayer side="before" noting={props.noting} clock={props.clock} />
            </Frame>
          </figure>
          <figure>
            <figcaption title={props.afterLabel} data-caption="after">{props.afterWord}</figcaption>
            <Frame size={props.size}>
              {media("after")}
              {marks()}
              <NoteLayer side="after" noting={props.noting} clock={props.clock} />
            </Frame>
          </figure>
        </div>
      </Match>
      <Match when={props.mode === "slider"}>
        <Slider {...props} media={media} marks={marks} />
      </Match>
      <Match when={props.mode === "onion"}>
        <Onion {...props} media={media} marks={marks} />
      </Match>
      <Match when={props.mode === "diff"}>
        <figure class="layers one">
          <figcaption>Where they differ</figcaption>
          <Frame size={props.size} class="diff">
            <div class="blend">
              {media("before")}
              <div class="over">{media("after")}</div>
            </div>
            <div class="tint" />
            {marks()}
          </Frame>
        </figure>
      </Match>
      <Match when={props.mode === "blink"}>
        <Blink {...props} media={media} marks={marks} />
      </Match>
    </Switch>
  );
}

type Layered = {
  size: [number, number] | undefined;
  beforeLabel: string;
  afterLabel: string;
  beforeWord: string;
  afterWord: string;
  media: (side: Side) => JSX.Element;
  marks: () => JSX.Element;
};

function Slider(props: Layered) {
  const [at, setAt] = createSignal(50);
  const [glide, setGlide] = createSignal(false);
  let frame: HTMLDivElement | undefined;
  let dragging = false;
  const put = (v: number, smooth: boolean) => {
    setGlide(smooth);
    setAt(Math.max(0, Math.min(100, v)));
  };
  const fromPointer = (e: PointerEvent) => {
    // The layer the picture zooms on, so the handle lands under the pointer at any zoom.
    const r = (frame!.querySelector("[data-zoom-layer]") ?? frame!).getBoundingClientRect();
    return ((e.clientX - r.left) / r.width) * 100;
  };
  return (
    <figure class="layers one">
      <figcaption class="ends">
        <span title={props.beforeLabel} data-caption="before">{props.beforeWord}</span>
        <span title={props.afterLabel} data-caption="after">{props.afterWord}</span>
      </figcaption>
      <Frame
        size={props.size}
        class={`slide ${glide() ? "glide" : ""}`}
        ref={(el) => (frame = el)}
        onPointerDown={(e) => {
          dragging = true;
          frame!.setPointerCapture(e.pointerId);
          put(fromPointer(e), true);
        }}
        onPointerMove={(e) => dragging && put(fromPointer(e), false)}
        onPointerUp={() => (dragging = false)}
        onPointerCancel={() => (dragging = false)}
      >
        {props.media("before")}
        <div class="over" style={{ "clip-path": `inset(0 0 0 ${at()}%)` }}>
          {props.media("after")}
        </div>
        {props.marks()}
        <div
          class="handle"
          style={{ left: `${at()}%` }}
          role="slider"
          tabindex="0"
          aria-label="Where the before ends"
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={Math.round(at())}
          onKeyDown={(e) => {
            const step = e.shiftKey ? 10 : 2;
            if (e.key === "ArrowLeft") put(at() - step, true);
            else if (e.key === "ArrowRight") put(at() + step, true);
            else return;
            e.preventDefault();
          }}
        >
          <span class="grip" />
        </div>
      </Frame>
    </figure>
  );
}

function Onion(props: Layered) {
  const [show, setShow] = createSignal(50);
  return (
    <figure class="layers one">
      <figcaption>{props.afterWord} over {props.beforeWord.toLowerCase()}</figcaption>
      <Frame size={props.size}>
        {props.media("before")}
        <div class="over" style={{ opacity: show() / 100 }}>
          {props.media("after")}
        </div>
        {props.marks()}
      </Frame>
      <label class="range">
        <span data-caption="before">{props.beforeWord}</span>
        <input
          type="range"
          min="0"
          max="100"
          value={show()}
          aria-label="How much of the after shows"
          onInput={(e) => setShow(Number(e.currentTarget.value))}
        />
        <span data-caption="after">{props.afterWord}</span>
      </label>
    </figure>
  );
}

function Blink(props: Layered) {
  const [before, setBefore] = createSignal(false);
  const [held, setHeld] = createSignal(false);
  const timer = setInterval(() => held() || setBefore((b) => !b), 650);
  onCleanup(() => clearInterval(timer));
  return (
    <figure class="layers one">
      <figcaption title={before() ? props.beforeLabel : props.afterLabel} data-caption={before() ? "before" : "after"}>
        {before() ? props.beforeWord : props.afterWord}
      </figcaption>
      <Frame
        size={props.size}
        class={`blink ${before() ? "show-before" : ""}`}
        onPointerEnter={() => setHeld(true)}
        onPointerLeave={() => setHeld(false)}
      >
        {props.media("before")}
        <div class="over">{props.media("after")}</div>
        {props.marks()}
      </Frame>
      <p class="hint">Hold the pointer on it to stop on one side.</p>
    </figure>
  );
}

/** Both films' lanes on one clock: grey where the pictures match, the change colour where
 * they differ, a playhead, and a jump to each moment they differ. A film alone (no `film`
 * comparison) has its one lane, on its `side`. */
function Timeline(props: { film?: Comparison | undefined; pair?: boolean; side?: Side; clock: Clock; words?: [string, string] }) {
  const total = () => props.clock.total();
  const words = () => props.words ?? ["Before", "After"];
  let lane: HTMLDivElement | undefined;
  const at = (e: PointerEvent) => {
    const r = lane!.getBoundingClientRect();
    props.clock.seek(((e.clientX - r.left) / r.width) * total());
  };
  const pct = (s: number) => `${(s / total()) * 100}%`;
  const sides = (): (readonly [Side, string, number])[] =>
    props.film
      ? [
          ["before", words()[0], props.film.total_before],
          ["after", words()[1], props.film.total_after],
        ]
      : props.pair
        ? [
            ["before", words()[0], 0],
            ["after", words()[1], 0],
          ]
        : [[props.side ?? "after", props.side === "before" ? words()[0] : words()[1], 0]];
  const segments = (side: Side, end: number) =>
    props.film ? lanes(props.film.pairs, side, end) : [{ start: 0, stop: Number.POSITIVE_INFINITY, differs: false }];
  // Each lane as drawn, kept while what it draws is the same: a live event that redraws the run,
  // or a film whose length becomes known, never draws the lanes again, which would play their
  // growing in again.
  const drawn = createMemo(
    () => sides().map(([side, label, end]) => ({ side, label, segments: segments(side, end) })),
    undefined,
    { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) },
  );
  return (
    <div class="film">
      <div class="clock">
        <button type="button" class="chip play" data-play title="Play or pause (Space)" onClick={() => props.clock.toggle()}>
          {props.clock.playing() ? "Pause" : props.film || props.pair ? "Play both" : "Play"}
        </button>
        <span class="t" data-clock>
          {props.clock.t().toFixed(2)} s
        </span>
        <div class="speeds seg" role="group" aria-label="Speed">
          <For each={SPEEDS}>
            {(x) => (
              <button type="button" data-speed={x} aria-pressed={speed() === x} title={`Play at ${x}× (< and > step it)`} onClick={() => setSpeed(x)}>
                {`${x}×`}
              </button>
            )}
          </For>
        </div>
        <div class="moments">
          <For each={props.film?.moments ?? []}>
            {(m) => (
              <button type="button" class="chip jump" data-seek={m.after[0]} onClick={() => props.clock.seek(m.after[0])}>
                {m.after[0].toFixed(2)} s
              </button>
            )}
          </For>
        </div>
      </div>
      <div
        class="timeline"
        data-measured={props.film ? "" : undefined}
        ref={lane}
        role="slider"
        tabindex="0"
        aria-label={props.film ? "Time in both films" : "Time in the film"}
        aria-valuemin="0"
        aria-valuemax={total().toFixed(2)}
        aria-valuenow={props.clock.t().toFixed(2)}
        onPointerDown={(e) => {
          lane!.setPointerCapture(e.pointerId);
          at(e);
        }}
        onPointerMove={(e) => e.buttons && at(e)}
        onKeyDown={(e) => {
          const step = e.shiftKey ? 1 : 0.1;
          if (e.key === "ArrowRight") props.clock.seek(props.clock.t() + step);
          else if (e.key === "ArrowLeft") props.clock.seek(props.clock.t() - step);
          else return;
          e.preventDefault();
        }}
      >
        <For each={drawn()}>
          {({ side, label, segments }) => (
            <div class="lane" data-lane={side}>
              <For each={segments}>
                {(s) => (
                  <i
                    data-differs={String(s.differs)}
                    style={{ left: pct(s.start), width: Number.isFinite(s.stop) ? pct(Math.max(0, s.stop - s.start)) : "100%" }}
                  />
                )}
              </For>
              <em>{label}</em>
            </div>
          )}
        </For>
        <div class="playhead" style={{ left: pct(props.clock.t()) }} />
      </div>
    </div>
  );
}

/** A text of a look from a CLI that did not write its diff into the bundle: its before and
 * after read from the files origin, line by line. */
function TextDiff(props: { before: string; after: string }) {
  const read = async (url: string) => {
    if (!url) return "";
    const answer = await fetch(url);
    if (!answer.ok) throw new Error(`the files origin answered ${answer.status}`);
    return answer.text();
  };
  const [texts] = createResource(
    () => [props.before, props.after] as const,
    ([before, after]) => Promise.all([read(before), read(after)]),
  );
  return (
    <Show when={!texts.error} fallback={<p class="missing">The text could not be read.</p>}>
      <Show when={texts()} fallback={<p class="missing">Reading the text.</p>}>
        {(t) => <DiffLines was={t()[0]} now={t()[1]} context={3} />}
      </Show>
    </Show>
  );
}

/** The picture on the stage: what it is, how to compare it, and the comparison itself. */
export function Stage(props: {
  shown: Shown;
  files: string;
  head: string;
  mode: Mode;
  onMode: (mode: Mode) => void;
  checks: number;
  noting: Noting;
  /** Where a film's clock is, as it moves: the moment a note on the film marks. */
  onTime: (t: number | undefined) => void;
  /** How far the pictures are zoomed and where, shared by both sides and kept across pictures. */
  view: View;
  onView: (view: View) => void;
  /** Whether Space is held, so a drag moves a zoomed picture instead of drawing a note. */
  panning: boolean;
  /** Whether the stage and its review fill the screen. */
  full: boolean;
  onFull: () => void;
  /** What the left side shows in place of the base: an earlier run's picture, compared here. */
  against?: Against | undefined;
  /** The strip of runs to compare with, shown above the picture. */
  strip?: JSX.Element;
}) {
  const picture = () => props.shown.picture;
  const url = (path: string | null) => (path && props.files ? fileUrl(props.files, path) : "");
  const film = () => isFilm(picture());
  const text = () => isText(picture());
  // An earlier run's picture is compared in the page; until it is, there is no comparison.
  const comparison = () => (props.against ? (props.against.comparison ?? null) : picture().comparison);
  /** Whether the page still compares the picture with an earlier run. */
  const comparing = () => !!props.against && !props.against.comparison && !props.against.error;
  const size = createMemo((): [number, number] | undefined => {
    const c = comparison();
    if (!c) return undefined;
    const [w, h] = c.size_after;
    return w > 0 && h > 0 ? [w, h] : undefined;
  });
  const both = () => (props.against != null || picture().before != null) && picture().after != null;
  const base = () => short(props.shown.look.base_commit);
  const beforeLabel = () => props.against?.label ?? `Before, ${base()}`;
  const afterLabel = () => `${props.against ? "Now" : "After"}, ${short(props.head)}`;
  const beforeWord = () => props.against?.word ?? "Before";
  const afterWord = () => (props.against ? "Now" : "After");
  const beforeUrl = () => (props.against ? props.against.url : url(picture().before));
  return (
    <article
      class="stage"
      data-stage
      data-key={props.shown.key}
      data-mode={props.mode}
      data-change={picture().change}
      data-flaky={String(!!picture().flaky)}
      aria-label={title(props.shown.parts)}
      data-panning={props.panning && props.view.s > 1 ? "" : undefined}
      data-against={props.against ? (props.against.look ? "look" : "run") : undefined}
    >
      <Zoom.Provider value={{ view: () => props.view, onView: (v) => props.onView(v), panning: () => props.panning }}>
      <header class="stage-head">
        <div class="named">
        <h3>{props.shown.parts.screen}</h3>
        <Show when={props.shown.parts.viewport}>
          <span class="tag">{props.shown.parts.viewport}</span>
        </Show>
        <Show when={props.shown.parts.theme}>
          <span class="tag">{props.shown.parts.theme}</span>
        </Show>
        <Show when={props.checks > 1}>
          <span class="tag">{props.shown.check}</span>
        </Show>
        <span
          class={`word ${picture().change}`}
          title={`${plainSummary(picture())} ${props.shown.picture.name}, taken by ${props.shown.check}, compared with the same check on ${base()}.${props.shown.look.note ? ` ${props.shown.look.note}` : ""}`}
        >
          {film() ? `${WORDS[picture().change]} film` : WORDS[picture().change]}
        </span>
        </div>
        <div class="tools">
        <Show when={!text()}>
          <div class="zoom seg" role="group" aria-label="Zoom">
            <button type="button" data-zoom="out" title="Zoom out (-)" disabled={props.view.s <= 1} onClick={() => props.onView(stepped(props.view, -1))}>
              −
            </button>
            <button type="button" data-zoom="fit" class="num" title="The whole picture (0); pinch or ⌘ scroll to zoom where the pointer is" onClick={() => props.onView(FIT)}>
              {`${Math.round(props.view.s * 100)}%`}
            </button>
            <button type="button" data-zoom="in" title="Zoom in (+)" onClick={() => props.onView(stepped(props.view, 1))}>
              +
            </button>
          </div>
          <button
            type="button"
            class="full-screen"
            data-full-screen
            aria-pressed={props.full}
            aria-label={props.full ? "Back to the page" : "Full screen"}
            title={props.full ? "Back to the page (Esc)" : "Full screen (f)"}
            onClick={() => props.onFull()}
          >
            <Icon name={props.full ? "shrink" : "expand"} />
            <span>{props.full ? "Back" : "Full screen"}</span>
          </button>
        </Show>
        <Show when={both() && !text()}>
          <div class="modes seg" role="group" aria-label="How to compare">
            <For each={MODES}>
              {([key, label, brief], k) => (
                <button type="button" data-mode={key} aria-pressed={props.mode === key} title={`${label} (${k() + 1})`} onClick={() => props.onMode(key)}>
                  <span class="long">{label}</span>
                  <span class="brief" aria-hidden="true">
                    {brief}
                  </span>
                </button>
              )}
            </For>
          </div>
        </Show>
        </div>
      </header>
      {props.strip}
      <Show when={props.against}>
        {(a) => (
          <p class="pair-says" data-pair-says={a().error ? "error" : a().comparison ? a().comparison!.verdict : "comparing"} aria-live="polite">
            <Show when={!a().error} fallback={<>Could not compare here: {a().error}</>}>
              <Show when={a().comparison} fallback={<>Comparing with {a().word.toLowerCase()}…</>}>
                {(c) => <>{pairSummary(c(), a().word)}</>}
              </Show>
            </Show>
          </p>
        )}
      </Show>
      <Show when={picture().flaky}>
        <p class="flaky-note" data-flaky-note title="This picture changes from run to run, so it is shown but never waits for you">
          Flaky, never waits for you
        </p>
      </Show>
      <Show
        when={!text()}
        fallback={
          <Show when={picture().diff} fallback={<TextDiff before={url(picture().before)} after={url(picture().after)} />}>
            {(lines) => <DiffLines lines={lines()} />}
          </Show>
        }
      >
      <Show
        when={both()}
        fallback={
          <figure class="layers one">
            <figcaption title={picture().after ? afterLabel() : beforeLabel()}>{picture().after ? "After" : "Before"}</figcaption>
            <Show when={picture().after ?? picture().before} fallback={<p class="missing">Neither side could be read.</p>}>
              {(path) => {
                const side = () => (picture().after ? "after" : "before");
                const label = () => (picture().after ? afterLabel() : beforeLabel());
                return (
                  <Show
                    when={film() && path()}
                    keyed
                    fallback={
                      <Frame size={size()}>
                        <Media src={url(path())} side={side()} clock={undefined} label={label()} />
                        <NoteLayer side={side()} noting={props.noting} />
                      </Frame>
                    }
                  >
                    {(alone) => {
                      // In full screen, a film stepped to plays at once.
                      const full = untrack(() => props.full);
                      const clock = createClock(() => 0.01, () => full);
                      createEffect(() => props.onTime(clock.t()));
                      onCleanup(() => props.onTime(undefined));
                      return (
                        <>
                          <Frame size={size()}>
                            <Media src={url(alone)} side={side()} clock={clock} label={label()} />
                            <NoteLayer side={side()} noting={props.noting} clock={clock} />
                          </Frame>
                          <Timeline side={side()} clock={clock} />
                        </>
                      );
                    }}
                  </Show>
                );
              }}
            </Show>
          </figure>
        }
      >
        <Show
          when={film()}
          fallback={
            <Compare
              mode={props.mode}
              before={beforeUrl()}
              after={url(picture().after)}
              size={size()}
              boxes={comparison() ? stillMarks(comparison()!) : []}
              clock={undefined}
              beforeLabel={beforeLabel()}
              afterLabel={afterLabel()}
              beforeWord={beforeWord()}
              afterWord={afterWord()}
              noting={props.noting}
            />
          }
        >
          {/* Both films on one clock, made once per pair, so the comparison arriving only adds
              lanes and outlines. Until both films can play through and are compared they show
              dimmed at their first frame, with what the page is doing. */}
          <Show when={`${beforeUrl()}\n${url(picture().after)}`} keyed>
            {(_) => {
              const measured = () => comparison() ?? undefined;
              const full = untrack(() => props.full);
              const clock = createClock(() => {
                const m = measured();
                return m ? Math.max(m.total_before, m.total_after) : 0;
              }, () => full && !comparing());
              const sameSize = () => {
                const m = measured();
                return !!m && m.size_before[0] === m.size_after[0] && m.size_before[1] === m.size_after[1];
              };
              const busy = () => (!clock.ready() ? "Loading both films…" : comparing() ? `Comparing with ${props.against!.word.toLowerCase()}…` : undefined);
              createEffect(() => props.onTime(clock.t()));
              onCleanup(() => props.onTime(undefined));
              return (
                <>
                  <div class="busy-wrap" data-busy={busy() ? "" : undefined}>
                    <Compare
                      mode={props.mode}
                      before={beforeUrl()}
                      after={url(picture().after)}
                      size={size()}
                      boxes={sameSize() ? marksAt(measured()!, clock.t()) : []}
                      clock={clock}
                      beforeLabel={beforeLabel()}
                      afterLabel={afterLabel()}
                      beforeWord={beforeWord()}
                      afterWord={afterWord()}
                      noting={props.noting}
                    />
                    <Show when={busy()}>
                      {(says) => (
                        <p class="busy-says" data-busy-says role="status">
                          <i aria-hidden="true" />
                          {says()}
                        </p>
                      )}
                    </Show>
                  </div>
                  <Timeline film={measured()} pair side="after" clock={clock} words={[beforeWord(), afterWord()]} />
                </>
              );
            }}
          </Show>
        </Show>
      </Show>
      </Show>
      </Zoom.Provider>
    </article>
  );
}
