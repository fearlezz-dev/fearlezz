// The page's comparing worker: it decodes two stills or films the way the CLI reads them and
// compares them with the CLI's own comparison (fearlezz-compare, built to wasm from
// ee/compare), off the page's thread. A still decodes through createImageBitmap to the RGBA of a
// canvas, without colour conversion; a film through WebCodecs, as the I420 planes ffmpeg's
// yuv420p equals, each frame copied straight into the wasm memory. A film is decoded once: the
// wasm side keeps the frames that start its states for the comparison's second read.
//
// Answers are kept per pair of files by their SHA-256 and their marks, so the same pair is
// compared once however its URLs change (a grant's folder lasts an hour).

import wasmUrl from "fearlezz-page-compare.wasm?url";
import { codecOf, demux } from "./demux";

/** One side of an ask: the file's URL and, for a film, the JSON of what its check said happened
 * in it, which the comparison times it from. */
export type Side = { url: string; marks: string | null };
/** What the page asks: compare the file at `before` with the one at `after`, for the picture
 * shown or, `ahead`, for one a person likely shows next. */
export type Ask = { id: number; before: Side; after: Side; film: boolean; ahead?: boolean };
/** The page now shows the picture of an ask it made ahead. */
export type Promote = { promote: number };
/** What the worker answers: the comparison as the CLI's JSON, or why there is none. */
export type Answer = { id: number; json: string } | { id: number; error: string };

/** The exports of ee/compare (see its lib.rs). */
type Compare = {
  memory: WebAssembly.Memory;
  input(len: number): number;
  film(w: number, h: number, frames: number, yuv: number): number;
  frame(id: number, start: number, dur: number): number;
  marks(id: number): number;
  compared(before: number, after: number): number;
  forget(id: number): void;
  output(): number;
  output_len(): number;
};

/** How long ffprobe says a still lasts (image2 at 25 frames a second), so totals match. */
const STILL_SECONDS = 0.04;

let loading: Promise<Compare> | undefined;
const load = () =>
  (loading ??= (async () => {
    const answer = await fetch(wasmUrl);
    if (!answer.ok) throw new Error(`the comparison could not load (${answer.status})`);
    const { instance } = await WebAssembly.instantiate(await answer.arrayBuffer(), {});
    return instance.exports as unknown as Compare;
  })());

/** The last answer of the wasm side, as text. */
const said = (w: Compare) => new TextDecoder().decode(new Uint8Array(w.memory.buffer, w.output(), w.output_len()));

/** Fails with the wasm side's words when a call says it failed. */
function check(w: Compare, status: number) {
  if (status !== 0) throw new Error(said(w));
}

/** Room for `len` bytes in the wasm memory, as a view to write the next frame into. */
function room(w: Compare, len: number): Uint8Array {
  // Asked first: making room may grow the memory, which replaces its buffer.
  const at = w.input(len);
  return new Uint8Array(w.memory.buffer, at, len);
}

async function feedStill(w: Compare, bytes: ArrayBuffer): Promise<number> {
  const bitmap = await createImageBitmap(new Blob([bytes]), { colorSpaceConversion: "none", premultiplyAlpha: "none" });
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("the page could not draw the still");
  context.drawImage(bitmap, 0, 0);
  const pixels = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
  const id = w.film(bitmap.width, bitmap.height, 1, 0);
  room(w, pixels.length).set(pixels);
  check(w, w.frame(id, 0, STILL_SECONDS));
  return id;
}

async function feedFilm(w: Compare, bytes: ArrayBuffer): Promise<number> {
  const { codec, w: width, h: height, blocks } = demux(bytes);
  const id = w.film(width, height, blocks.length, 1);
  const cw = Math.ceil(width / 2);
  const ch = Math.ceil(height / 2);
  const len = width * height + 2 * cw * ch;
  const layout = [
    { offset: 0, stride: width },
    { offset: width * height, stride: cw },
    { offset: width * height + cw * ch, stride: cw },
  ];
  // Frames come out in the order they are shown, which is the order of `blocks`; each is
  // copied and fed before the next, since the wasm side has one frame of room.
  let shown = 0;
  let chain = Promise.resolve();
  let failed: unknown;
  const decoder = new VideoDecoder({
    output: (frame) => {
      const block = blocks[shown++];
      chain = chain
        .then(async () => {
          if (failed) return;
          if (!block) throw new Error("the film gave more frames than it holds");
          if (frame.format !== "I420") throw new Error(`the page compares yuv420p films, not ${frame.format ?? "this format"}`);
          await frame.copyTo(room(w, len), { rect: { x: 0, y: 0, width, height }, layout });
          check(w, w.frame(id, block.start, block.dur));
        })
        .catch((e: unknown) => {
          failed ??= e;
        })
        .finally(() => frame.close());
    },
    error: (e) => {
      failed ??= e;
    },
  });
  try {
    decoder.configure({ codec: codecOf(codec), codedWidth: width, codedHeight: height, hardwareAcceleration: "prefer-software" });
    for (const block of blocks) {
      if (failed) break;
      decoder.decode(new EncodedVideoChunk({ type: block.key ? "key" : "delta", timestamp: Math.round(block.start * 1e6), duration: Math.round(block.dur * 1e6), data: block.data }));
      // A short queue, so frames are copied and closed as they come.
      while (decoder.decodeQueueSize > 8) await new Promise((r) => decoder.addEventListener("dequeue", r, { once: true }));
    }
    if (!failed) await decoder.flush();
    await chain;
  } catch (e) {
    failed ??= e;
  } finally {
    if (decoder.state !== "closed") decoder.close();
  }
  if (!failed && shown !== blocks.length) failed = new Error(`the film decoded to ${shown} of its ${blocks.length} frames`);
  if (failed) {
    w.forget(id);
    throw failed;
  }
  return id;
}

async function sha256(bytes: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return [...digest].map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function fetched(url: string): Promise<ArrayBuffer> {
  // As the page fetches films ahead (preload.ts), so a film fetched then is in the cache.
  const answer = await fetch(url, { mode: "cors", credentials: "omit" });
  if (!answer.ok) throw new Error(`the files origin answered ${answer.status}`);
  return answer.arrayBuffer();
}

const known = new Map<string, string>();

async function compare(ask: Ask): Promise<string> {
  const [w, before, after] = await Promise.all([load(), fetched(ask.before.url), fetched(ask.after.url)]);
  const key = JSON.stringify([await sha256(before), await sha256(after), ask.before.marks, ask.after.marks]);
  const kept = known.get(key);
  if (kept) return kept;
  const feed = async (bytes: ArrayBuffer, side: Side) => {
    const id = await (ask.film ? feedFilm(w, bytes) : feedStill(w, bytes));
    if (side.marks) {
      const json = new TextEncoder().encode(side.marks);
      room(w, json.length).set(json);
      try {
        check(w, w.marks(id));
      } catch (e) {
        w.forget(id);
        throw e;
      }
    }
    return id;
  };
  const b = await feed(before, ask.before);
  let a: number;
  try {
    a = await feed(after, ask.after);
  } catch (e) {
    w.forget(b);
    throw e;
  }
  check(w, w.compared(b, a));
  const json = said(w);
  known.set(key, json);
  return json;
}

// One comparison at a time: the wasm side has one frame of room and one answer. The page shows
// one picture, so a newer ask takes the place of one that has not started yet. Asks made ahead
// wait behind it, the oldest dropped past `AHEAD`, and one the page comes to show moves up.
const scope = self as unknown as { onmessage: ((e: MessageEvent<Ask | Promote>) => void) | null; postMessage: (answer: Answer) => void };
const AHEAD = 6;
let running = false;
let next: Ask | undefined;
const ahead: Ask[] = [];
const shown = (ask: Ask) => {
  if (next) scope.postMessage({ id: next.id, error: "a newer comparison took its place" });
  next = ask;
};
scope.onmessage = (e) => {
  const m = e.data;
  if ("promote" in m) {
    const at = ahead.findIndex((a) => a.id === m.promote);
    if (at >= 0) shown(ahead.splice(at, 1)[0]!);
  } else if (m.ahead) {
    ahead.push(m);
    while (ahead.length > AHEAD) scope.postMessage({ id: ahead.shift()!.id, error: "no longer ahead" });
  } else shown(m);
  void drain();
};

async function drain() {
  if (running) return;
  running = true;
  for (let ask = next ?? ahead.shift(); ask; ask = next ?? ahead.shift()) {
    if (ask === next) next = undefined;
    try {
      scope.postMessage({ id: ask.id, json: await compare(ask) });
    } catch (error) {
      // A trap (a panic or memory that ran out) leaves the wasm side's state half changed, so the
      // next comparison starts a new instance.
      if (error instanceof WebAssembly.RuntimeError) loading = undefined;
      scope.postMessage({ id: ask.id, error: error instanceof Error ? error.message : String(error) });
    }
  }
  running = false;
}
