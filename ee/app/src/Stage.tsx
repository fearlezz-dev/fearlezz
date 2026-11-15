// One picture on the light table: its before and after, the five ways to compare them, and for
// a film both films on one clock with lanes of where they differ.

import { type JSX, For, Match, Show, Switch, createMemo, createSignal, onCleanup, splitProps } from "solid-js";
import { fileUrl } from "./grant";
import { type Box, type Shown, WORDS, isFilm, lanes, marksAt, plainSummary, stillMarks, title } from "./pictures";
import type { Comparison } from "./proof";

export const MODES = [
  ["side", "Side by side"],
  ["slider", "Slider"],
  ["onion", "Onion skin"],
  ["diff", "Difference"],
  ["blink", "Blink"],
] as const;

export type Mode = (typeof MODES)[number][0];

type Side = "before" | "after";

/** The films' one clock: where both are, whether they play, and the videos it moves. */
type Clock = {
  t: () => number;
  playing: () => boolean;
  total: number;
  add: (video: HTMLVideoElement) => void;
  seek: (t: number) => void;
  toggle: () => void;
};

function createClock(total: number): Clock {
  const [t, setT] = createSignal(0);
  const [playing, setPlaying] = createSignal(false);
  const videos = new Set<HTMLVideoElement>();
  let frame = 0;
  // A film shorter than the clock rests on its last frame.
  const end = (v: HTMLVideoElement) => (Number.isFinite(v.duration) ? v.duration : total) - 0.01;
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
    const clamped = Math.max(0, Math.min(total, at));
    setT(clamped);
    for (const v of videos) place(v, clamped);
  };
  const play = () => {
    if (t() >= total - 0.05) seek(0);
    setPlaying(true);
    const started = performance.now() - t() * 1000;
    const tick = (now: number) => {
      const at = (now - started) / 1000;
      if (at >= total) {
        setT(total);
        stop();
        return;
      }
      setT(at);
      for (const v of videos) {
        if (at < end(v)) {
          if (v.paused) void v.play().catch(() => {});
          // Films drift apart as they play; one that falls behind the clock is put back.
          if (Math.abs(v.currentTime - at) > 0.15) v.currentTime = at;
        } else if (!v.paused) v.pause();
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
  };
  onCleanup(stop);
  return {
    t,
    playing,
    total,
    add: (video) => {
      videos.add(video);
      video.addEventListener("loadedmetadata", () => place(video, t()));
      onCleanup(() => videos.delete(video));
    },
    seek,
    toggle: () => (playing() ? stop() : play()),
  };
}

/** One side of a picture: an image, or a video the clock moves. */
function Media(props: { src: string; side: Side; clock: Clock | undefined; label: string }) {
  return (
    <Show
      when={props.clock}
      fallback={<img src={props.src} alt={props.label} data-side={props.side} draggable={false} decoding="async" />}
    >
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

/** A frame as large as its picture shows, so outlines sit on the picture and not on its mat. */
function Frame(
  props: { size: [number, number] | undefined; class?: string; children: JSX.Element } & Omit<
    JSX.HTMLAttributes<HTMLDivElement>,
    "class" | "style" | "children"
  >,
) {
  const [own, rest] = splitProps(props, ["size", "class", "children"]);
  const style = () => {
    const size = own.size;
    return size ? { "--ratio": `${size[0]} / ${size[1]}`, "--w": `${size[0]}`, "--h": `${size[1]}` } : {};
  };
  return (
    <div {...rest} class={`frame ${own.size ? "sized" : ""} ${own.class ?? ""}`} style={style()}>
      {own.children}
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
            <figcaption>{props.beforeLabel}</figcaption>
            <Frame size={props.size}>
              {media("before")}
              {marks()}
            </Frame>
          </figure>
          <figure>
            <figcaption>{props.afterLabel}</figcaption>
            <Frame size={props.size}>
              {media("after")}
              {marks()}
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
    const r = frame!.getBoundingClientRect();
    return ((e.clientX - r.left) / r.width) * 100;
  };
  return (
    <figure class="layers one">
      <figcaption class="ends">
        <span>{props.beforeLabel}</span>
        <span>{props.afterLabel}</span>
      </figcaption>
      <Frame
        size={props.size}
        class={`slide ${glide() ? "glide" : ""}`}
        ref={frame}
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
      <figcaption>The after over the before</figcaption>
      <Frame size={props.size}>
        {props.media("before")}
        <div class="over" style={{ opacity: show() / 100 }}>
          {props.media("after")}
        </div>
        {props.marks()}
      </Frame>
      <label class="range">
        <span>Before</span>
        <input
          type="range"
          min="0"
          max="100"
          value={show()}
          aria-label="How much of the after shows"
          onInput={(e) => setShow(Number(e.currentTarget.value))}
        />
        <span>After</span>
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
      <figcaption>{before() ? props.beforeLabel : props.afterLabel}</figcaption>
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
 * they differ, a playhead, and a jump to each moment they differ. */
function Timeline(props: { film: Comparison; clock: Clock }) {
  const total = () => props.clock.total;
  let lane: HTMLDivElement | undefined;
  const at = (e: PointerEvent) => {
    const r = lane!.getBoundingClientRect();
    props.clock.seek(((e.clientX - r.left) / r.width) * total());
  };
  const pct = (s: number) => `${(s / total()) * 100}%`;
  const sides = [
    ["before", "Before", props.film.total_before],
    ["after", "After", props.film.total_after],
  ] as const;
  return (
    <div class="film">
      <div class="clock">
        <button type="button" class="chip play" onClick={() => props.clock.toggle()}>
          {props.clock.playing() ? "Pause" : "Play both"}
        </button>
        <span class="t" data-clock>
          {props.clock.t().toFixed(2)} s
        </span>
        <For each={props.film.moments}>
          {(m) => (
            <button type="button" class="chip jump" data-seek={m.after[0]} onClick={() => props.clock.seek(m.after[0])}>
              {m.after[0].toFixed(2)} s
            </button>
          )}
        </For>
      </div>
      <div
        class="timeline"
        ref={lane}
        role="slider"
        tabindex="0"
        aria-label="Time in both films"
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
        <For each={sides}>
          {([side, label, end]) => (
            <div class="lane" data-lane={side}>
              <For each={lanes(props.film.pairs, side, end)}>
                {(s) => (
                  <i
                    data-differs={String(s.differs)}
                    style={{ left: pct(s.start), width: pct(Math.max(0, s.stop - s.start)) }}
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

/** The picture on the stage: what it is, how to compare it, and the comparison itself. */
export function Stage(props: {
  shown: Shown;
  files: string;
  head: string;
  mode: Mode;
  onMode: (mode: Mode) => void;
  position: number;
  of: number;
  onStep: (by: number) => void;
  checks: number;
}) {
  const picture = () => props.shown.picture;
  const url = (path: string | null) => (path && props.files ? fileUrl(props.files, path) : "");
  const film = () => isFilm(picture());
  const comparison = () => picture().comparison;
  const size = createMemo((): [number, number] | undefined => {
    const c = comparison();
    if (!c) return undefined;
    const [w, h] = c.size_after;
    return w > 0 && h > 0 ? [w, h] : undefined;
  });
  const both = () => picture().before != null && picture().after != null;
  const base = () => props.shown.look.base_commit.slice(0, 7);
  const beforeLabel = () => `Before, ${base()}`;
  const afterLabel = () => `After, ${props.head.slice(0, 7)}`;
  return (
    <article
      class="stage"
      data-stage
      data-key={props.shown.key}
      data-mode={props.mode}
      data-change={picture().change}
      data-flaky={String(!!picture().flaky)}
      aria-label={title(props.shown.parts)}
    >
      <header class="stage-head">
        <h3>{props.shown.parts.screen}</h3>
        <Show when={props.shown.parts.viewport}>
          <span class="tag">{props.shown.parts.viewport}</span>
        </Show>
        <Show when={props.shown.parts.theme}>
          <span class="tag">{props.shown.parts.theme}</span>
        </Show>
        <Show when={film()}>
          <span class="tag">Film</span>
        </Show>
        <Show when={props.checks > 1}>
          <span class="tag">{props.shown.check}</span>
        </Show>
        <span class={`word ${picture().change}`}>{WORDS[picture().change]}</span>
        <span class="step">
          <button type="button" class="ghost" aria-label="Previous picture" onClick={() => props.onStep(-1)} disabled={props.position <= 1}>
            ‹
          </button>
          <span class="num">
            {props.position} of {props.of}
          </span>
          <button type="button" class="ghost" aria-label="Next picture" onClick={() => props.onStep(1)} disabled={props.position >= props.of}>
            ›
          </button>
        </span>
      </header>
      <Show when={picture().flaky}>
        <p class="flaky-note" data-flaky-note>
          Flaky: this picture changes from run to run, so it is shown but never waits for you.
        </p>
      </Show>
      <Show when={props.shown.look.no_before}>
        {(why) => <p class="look-note">No before: {why()}. Every picture of this check counts as new.</p>}
      </Show>
      <Show
        when={both()}
        fallback={
          <figure class="layers one">
            <figcaption>{picture().after ? afterLabel() : beforeLabel()}</figcaption>
            <Show when={picture().after ?? picture().before} fallback={<p class="missing">Neither side could be read.</p>}>
              {(path) => (
                <Frame size={size()}>
                  <Media src={url(path())} side={picture().after ? "after" : "before"} clock={undefined} label={picture().after ? afterLabel() : beforeLabel()} />
                </Frame>
              )}
            </Show>
          </figure>
        }
      >
        <div class="modes seg" role="group" aria-label="How to compare">
          <For each={MODES}>
            {([key, label], k) => (
              <button type="button" data-mode={key} aria-pressed={props.mode === key} onClick={() => props.onMode(key)}>
                {label}
                <kbd>{k() + 1}</kbd>
              </button>
            )}
          </For>
        </div>
        <Show
          when={film() && comparison()}
          keyed
          fallback={
            <Compare
              mode={props.mode}
              before={url(picture().before)}
              after={url(picture().after)}
              size={size()}
              boxes={comparison() ? stillMarks(comparison()!) : []}
              clock={undefined}
              beforeLabel={beforeLabel()}
              afterLabel={afterLabel()}
            />
          }
        >
          {(c) => {
            const clock = createClock(Math.max(c.total_before, c.total_after, 0.01));
            const sameSize = c.size_before[0] === c.size_after[0] && c.size_before[1] === c.size_after[1];
            return (
              <>
                <Compare
                  mode={props.mode}
                  before={url(picture().before)}
                  after={url(picture().after)}
                  size={size()}
                  boxes={sameSize ? marksAt(c, clock.t()) : []}
                  clock={clock}
                  beforeLabel={beforeLabel()}
                  afterLabel={afterLabel()}
                />
                <Timeline film={c} clock={clock} />
              </>
            );
          }}
        </Show>
      </Show>
      <details class="more">
        <summary>Details</summary>
        <p>{plainSummary(picture())}</p>
        <p>
          {props.shown.picture.name}, taken by {props.shown.check}, compared with the same check on {props.shown.look.base_commit.slice(0, 10)}.
        </p>
        <Show when={props.shown.look.note}>{(note) => <p>{note()}</p>}</Show>
      </details>
    </article>
  );
}
