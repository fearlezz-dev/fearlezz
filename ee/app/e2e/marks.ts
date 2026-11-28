// What happens in a film of the host's pages, written to `proof.jsonl` for Fearlezz to time the
// film from: a person's every key and pointer press as a mark, and as free spans the page loading
// and settling (anything animating, or a part saying it is busy), and moving that no animation
// shows: the pointer travelling or dragging, typing into a field, a scroll. Two runs of the same
// film load, wait on answers, catch a fade and show any of those halfway at different times;
// timed from what happened rather than from the film's start, and judged only where each
// settles, they still look the same.
//
// The page notes each press itself, as the browser takes it, and the test stamps it as it
// arrives (the page's own clock stands still, for steady pictures), so a mark sits within a
// few milliseconds of the frame the press shows on. Times are wall clock milliseconds, with
// the film's start as a `start` line.

import { appendFileSync } from "node:fs";
import type { Page } from "@playwright/test";

type Line = { kind: "mark"; name: string; wall: number } | { kind: "span"; name: string; wall_start: number; wall_end: number; timing: "free" };

/** A stretch of moving that no animation shows (the pointer travelling or dragging, letters
 * typed, the page scrolled), by its first and latest step. */
type Moving = { name: "pointer" | "drag" | "typing" | "scroll"; start: number; last: number };

type Film = { start: number; loaded?: number; settling?: number | undefined; moving?: Moving | undefined; lines: Line[] };

/** How long without a step ends a stretch of moving: a pointer held still, typing that stopped,
 * a scroll that came to rest. */
const STILL = 150;
/** How long after the page last moved its film may still show it: the film draws each frame a
 * frame or two after the page does. */
const SHOWS = 100;

const noted = new WeakMap<Page, Film>();

/** Starts noting what happens in the film of `page`, which started recording when the page
 * was made, just before this. */
export async function noting(page: Page): Promise<void> {
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r(0))));
  const film: Film = { start: Date.now(), lines: [] };
  noted.set(page, film);
  await page.exposeBinding("fearlezzMark", (_, name: string) => {
    stopped(film);
    film.lines.push({ kind: "mark", name, wall: Date.now() });
  });
  await page.exposeBinding("fearlezzStepped", (_, name: Moving["name"]) => {
    const now = Date.now();
    stopped(film, name);
    if (film.moving) film.moving.last = now;
    else film.moving = { name, start: now, last: now };
  });
  await page.exposeBinding("fearlezzSettling", (_, moving: boolean) => {
    if (moving) film.settling ??= Date.now();
    else settled(film);
  });
  await page.addInitScript(() => {
    const mark = (name: string) => (window as unknown as { fearlezzMark: (name: string) => void }).fearlezzMark(name);
    const stepped = (name: string) => (window as unknown as { fearlezzStepped: (name: string) => void }).fearlezzStepped(name);
    // Each frame, whether anything animates (a fade, a spinner, a transition) or a part says it is
    // busy; the page's own clock stands still, but its animation frames run.
    let moving = false;
    const settling = () => {
      const now = document.getAnimations().some((a) => a.playState === "running") || document.querySelector('[aria-busy="true"]') !== null;
      if (now !== moving) {
        moving = now;
        (window as unknown as { fearlezzSettling: (moving: boolean) => void }).fearlezzSettling(now);
      }
      requestAnimationFrame(settling);
    };
    requestAnimationFrame(settling);
    const typing = (target: EventTarget | null) =>
      target instanceof HTMLElement && (target.isContentEditable || target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement);
    addEventListener(
      "keydown",
      (e: KeyboardEvent) => {
        // Letters typed into a field are one stretch of typing, not a mark each; a chord is its
        // own, marked once at its key rather than at its modifier.
        if (e.repeat || ["Control", "Meta", "Shift", "Alt"].includes(e.key)) return;
        if (typing(e.target) && !e.ctrlKey && !e.metaKey) stepped("typing");
        else mark(`key ${[e.ctrlKey && "Control", e.metaKey && "Meta", e.key === " " ? "Space" : e.key].filter(Boolean).join("+")}`);
      },
      true,
    );
    addEventListener(
      "pointerdown",
      (e: PointerEvent) => {
        // Named only by a label the page gives on purpose: a button's words change with what
        // it shows (Look again, a count), which would make two runs' marks differ.
        const target = e.target instanceof Element ? e.target.closest("[aria-label], [title]") : null;
        const name = target?.getAttribute("aria-label") ?? target?.getAttribute("title");
        mark(name ? `press ${name}` : "press");
      },
      true,
    );
    // The pointer the film draws travels to what it presses, and with a button down it drags (a
    // region drawn, a slider pulled, a zoomed picture panned); a press is a mark, and the frames
    // until the pointer comes to rest are a stretch of moving.
    addEventListener("pointermove", (e: PointerEvent) => stepped(e.buttons !== 0 ? "drag" : "pointer"), true);
    // A scroll, smooth or not, is no animation, so settling never sees it.
    addEventListener("scroll", () => stepped("scroll"), true);
  });
}

/** Ends the stretch of moving in `film` as a free span, once it stood still long enough or
 * another kind of moving (`next`) began. */
function stopped(film: Film, next?: Moving["name"]): void {
  const m = film.moving;
  const now = Date.now();
  if (!m || (m.name === next && now - m.last < STILL)) return;
  film.lines.push({ kind: "span", name: m.name, wall_start: m.start, wall_end: Math.min(m.last + SHOWS, now), timing: "free" });
  film.moving = undefined;
}

/** Ends the stretch `film` was settling in, as a free span, which its film shows a moment longer. */
function settled(film: Film): void {
  if (film.settling === undefined) return;
  film.lines.push({ kind: "span", name: "settling", wall_start: film.settling, wall_end: Date.now() + SHOWS, timing: "free" });
  film.settling = undefined;
}

/** The page of `page`'s film has loaded: what came before may take longer or shorter. */
export function loaded(page: Page): void {
  const film = noted.get(page);
  if (film && film.loaded === undefined) film.loaded = Date.now();
}

/** Writes what happened in `page`'s film, kept at `film` below the proof folder `out`. */
export function write(page: Page, out: string, film: string): void {
  const f = noted.get(page);
  if (!f) return;
  settled(f);
  stopped(f);
  const lines: object[] = [{ kind: "start", film, wall: f.start }];
  if (f.loaded !== undefined) lines.push({ kind: "span", film, name: "loading", wall_start: f.start, wall_end: f.loaded, timing: "free" });
  for (const line of f.lines) lines.push({ ...line, film });
  // One write, so films kept at once by several workers never interleave their lines.
  appendFileSync(`${out}/proof.jsonl`, `${lines.map((l) => JSON.stringify(l)).join("\n")}\n`);
}
