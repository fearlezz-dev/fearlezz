// What happens in a film of the host's pages, written to `proof.jsonl` for Fearlezz to time the
// film from: a person's every key and pointer press as a mark, and the page loading as a span
// whose timing may change. Two runs of the same film load and wait on answers for different
// times; timed from what happened rather than from the film's start, they still look the same.
//
// The page notes each press itself, as the browser takes it, and the test stamps it as it
// arrives (the page's own clock stands still, for steady pictures), so a mark sits within a
// few milliseconds of the frame the press shows on. Times are wall clock milliseconds, with
// the film's start as a `start` line.

import { appendFileSync } from "node:fs";
import type { Page } from "@playwright/test";

type Line = { kind: "mark"; name: string; wall: number } | { kind: "span"; name: string; wall_start: number; wall_end: number; timing: "free" };

const noted = new WeakMap<Page, { start: number; loaded?: number; lines: Line[] }>();

/** Starts noting what happens in the film of `page`, which started recording when the page
 * was made, just before this. */
export async function noting(page: Page): Promise<void> {
  const film = { start: Date.now(), lines: [] as Line[] };
  noted.set(page, film);
  await page.exposeBinding("fearlezzMark", (_, name: string) => {
    film.lines.push({ kind: "mark", name, wall: Date.now() });
  });
  await page.addInitScript(() => {
    const mark = (name: string) => (window as unknown as { fearlezzMark: (name: string) => void }).fearlezzMark(name);
    const typing = (target: EventTarget | null) =>
      target instanceof HTMLElement && (target.isContentEditable || target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement);
    addEventListener(
      "keydown",
      (e: KeyboardEvent) => {
        // Letters typed into a field are one step, not one each; a chord is its own, marked once
        // at its key rather than at its modifier.
        if (e.repeat || ["Control", "Meta", "Shift", "Alt"].includes(e.key) || (typing(e.target) && !e.ctrlKey && !e.metaKey)) return;
        mark(`key ${[e.ctrlKey && "Control", e.metaKey && "Meta", e.key === " " ? "Space" : e.key].filter(Boolean).join("+")}`);
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
  });
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
  const lines: object[] = [{ kind: "start", film, wall: f.start }];
  if (f.loaded !== undefined) lines.push({ kind: "span", film, name: "loading", wall_start: f.start, wall_end: f.loaded, timing: "free" });
  for (const line of f.lines) lines.push({ ...line, film });
  // One write, so films kept at once by several workers never interleave their lines.
  appendFileSync(`${out}/proof.jsonl`, `${lines.map((l) => JSON.stringify(l)).join("\n")}\n`);
}
