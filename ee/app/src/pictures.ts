// The pictures of a run as a person looks through them: every look's stills and films, in the
// order that directs attention (what changed first, what stayed the same folded last), grouped
// by the parts of their names, and what a film's lanes and outlines show at a time.

import type { Comparison, Footage, Look, Marks, Pair, Picture, PictureChange, Run, Section } from "./proof";

/** One picture of a run, by the check that took it. */
export type Shown = {
  /** Check and name: the same picture across runs. */
  key: string;
  check: string;
  /** The look it belongs to: what it was compared with. */
  look: Look;
  picture: Picture;
  parts: Parts;
};

/** A picture's name in parts: the viewport and theme it names, and the screen (the rest). */
export type Parts = { screen: string; viewport: string; theme: string };

/** The kinds of change in the order they are shown. */
export const ORDER: PictureChange[] = ["changed", "unreadable", "new", "gone", "timing", "same"];

export const WORDS: Record<PictureChange, string> = {
  changed: "Changed",
  unreadable: "Could not read",
  new: "New",
  gone: "Gone",
  timing: "Timing only",
  same: "Unchanged",
};

const VIEWPORTS = new Set([
  "desktop",
  "mobile",
  "phone",
  "tablet",
  "laptop",
  "wide",
  "narrow",
  "ipad",
  "iphone",
  "android",
  "watch",
]);
const THEMES = new Set(["light", "dark", "high-contrast", "contrast"]);

export function parts(name: string): Parts {
  // A file name shows without its extension.
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot + 1) : "";
  const bare = ext && !ext.includes("/") && ext.length <= 4 ? name.slice(0, dot) : name;
  const out: Parts = { screen: "", viewport: "", theme: "" };
  const rest: string[] = [];
  for (const segment of bare.split("/")) {
    const lower = segment.toLowerCase();
    if (!out.viewport && (VIEWPORTS.has(lower) || /^\d+x\d+$/.test(lower))) out.viewport = segment;
    else if (!out.theme && THEMES.has(lower)) out.theme = segment;
    else rest.push(segment);
  }
  out.screen = rest.length ? rest.join("/") : bare;
  return out;
}

const rank = (p: Shown) => ORDER.indexOf(p.picture.change) * 2 + (p.picture.flaky ? 1 : 0);

/** Every picture of the run's looks, in its checks' order: changed first, flaky ones after the
 * rest of their kind, each kind in the order its look lists them. */
export function picturesOf(run: Run): Shown[] {
  const all = run.checks.flatMap(({ id, look }) =>
    look
      ? look.items.map((picture) => ({
          key: `${id}/${picture.name}`,
          check: id,
          look,
          picture,
          parts: parts(picture.name),
        }))
      : [],
  );
  // Sort is stable, so each kind keeps its look's order.
  return all.sort((a, b) => rank(a) - rank(b));
}

/** What pictures can be grouped by. */
export type GroupBy = "change" | "check" | keyof Parts;

export const GROUP_WORDS: Record<GroupBy, string> = {
  change: "Change",
  check: "Check",
  screen: "Screen",
  viewport: "Viewport",
  theme: "Theme",
};

/** The groupings worth offering: by change always, and by anything else that differs. */
export function groupings(pictures: Shown[]): GroupBy[] {
  const differs = (of: (p: Shown) => string) => new Set(pictures.map(of).filter(Boolean)).size > 1;
  const out: GroupBy[] = ["change"];
  if (differs((p) => p.check)) out.push("check");
  for (const part of ["screen", "viewport", "theme"] as const) {
    if (differs((p) => p.parts[part])) out.push(part);
  }
  return out;
}

export type Group = {
  key: string;
  label: string;
  /** The kind of change all of it shares, when grouped by change. */
  change?: PictureChange;
  pictures: Shown[];
  /** Behind a click: the pictures that stayed the same, unless they are all there is. */
  folded: boolean;
};

/** The pictures in groups, filtered to one kind when `filter` names one. The same pictures
 * come last, folded, unless asked for or all there is. */
export function grouped(pictures: Shown[], by: GroupBy, filter: PictureChange | null): Group[] {
  const shown = filter ? pictures.filter((p) => p.picture.change === filter) : pictures;
  const allSame = shown.every((p) => p.picture.change === "same");
  const same = shown.filter((p) => p.picture.change === "same");
  const unchanged: Group = { key: "same", label: WORDS.same, change: "same", pictures: same, folded: !allSame };
  if (by === "change" || allSame) {
    return ORDER.map((change) => ({
      key: change,
      label: WORDS[change],
      change,
      pictures: shown.filter((p) => p.picture.change === change),
      folded: change === "same" && !allSame,
    })).filter((g) => g.pictures.length > 0);
  }
  const groups = new Map<string, Shown[]>();
  for (const p of shown) {
    if (p.picture.change === "same") continue;
    const value = (by === "check" ? p.check : p.parts[by]) || "Other";
    groups.set(value, [...(groups.get(value) ?? []), p]);
  }
  const out: Group[] = [...groups].map(([value, list]) => ({
    key: `${by}:${value}`,
    label: value,
    pictures: list,
    folded: false,
  }));
  return same.length ? [...out, unchanged] : out;
}

/** One screen of a check: its pictures that differ only in viewport and theme, its films
 * among them, since they show the same screen. */
export type Screen = {
  /** Check and screen. */
  key: string;
  label: string;
  /** Every variant, by viewport as first seen, films before stills (a film shows what its
   * still shows, and more) and light before dark within one. */
  variants: Shown[];
  /** The same variants per viewport, for showing them as a viewport and its themes. */
  viewports: { viewport: string; variants: Shown[] }[];
};

const themeRank = (theme: string) => (theme === "light" ? 0 : theme === "dark" ? 1 : 2);
const variantRank = (p: Shown) => (isFilm(p.picture) ? 0 : 3) + themeRank(p.parts.theme);

/** The pictures as screens, in the order each screen first comes, so the variants of a screen
 * show as one; j and k step through them in this same order. */
export function screensOf(pictures: Shown[]): Screen[] {
  const byKey = new Map<string, Shown[]>();
  for (const p of pictures) {
    const key = `${p.check}/${p.parts.screen}`;
    byKey.set(key, [...(byKey.get(key) ?? []), p]);
  }
  return [...byKey].map(([key, list]) => {
    const viewports: Screen["viewports"] = [];
    for (const p of list) {
      const at = viewports.find((v) => v.viewport === p.parts.viewport);
      if (at) at.variants.push(p);
      else viewports.push({ viewport: p.parts.viewport, variants: [p] });
    }
    for (const v of viewports) v.variants.sort((a, b) => variantRank(a) - variantRank(b));
    return { key, label: list[0]!.parts.screen, variants: viewports.flatMap((v) => v.variants), viewports };
  });
}

/** How many pictures of each kind there are, in order. */
export function counts(pictures: Shown[]): { change: PictureChange; n: number }[] {
  return ORDER.map((change) => ({ change, n: pictures.filter((p) => p.picture.change === change).length })).filter(
    (c) => c.n > 0,
  );
}

/** A box in full resolution pixels: x0, y0, x1, y1, with x1 and y1 exclusive. */
export type Box = [number, number, number, number];

/** A stretch of a film's lane, in seconds. */
export type Stretch = { start: number; stop: number; differs: boolean };

/** One film's lane: each run of pairs on the same state of that film, from its start to the
 * next state's start (or the film's `end`), differing when any of its pairs differs. */
export function lanes(pairs: Pair[], side: "before" | "after", end: number): Stretch[] {
  const runs: { state: number; at: number; differs: boolean }[] = [];
  for (const p of pairs) {
    const state = side === "before" ? p.before : p.after;
    const at = side === "before" ? p.before_at : p.after_at;
    const last = runs[runs.length - 1];
    if (last && last.state === state) last.differs ||= p.px > 0;
    else runs.push({ state, at, differs: p.px > 0 });
  }
  return runs.map((r, k) => ({ start: r.at, stop: runs[k + 1]?.at ?? end, differs: r.differs }));
}

/** Where a film differs at time `t` of its after film: the boxes of the state shown then. */
export function marksAt(film: Comparison, t: number): Box[] {
  let boxes: Box[] = [];
  for (const p of film.pairs) {
    if (p.after_at <= t + 0.001) boxes = p.boxes;
  }
  return boxes;
}

/** The films' timeline, as the comparison aligned it at their marks; empty for an older proof. */
type Timeline = Section[] | null | undefined;

const footage = (s: Section, side: "before" | "after"): Footage => (side === "before" ? s.before : s.after);

/** Where the films' timeline ends. */
export const timelineEnd = (timeline: Timeline): number => {
  const last = timeline?.[timeline.length - 1];
  return last ? last.at + last.length : 0;
};

/** A stretch with no footage shorter than this holds its frame quietly: shown, it would only
 * flicker. */
export const QUIET_GAP = 0.5;

/** Where one film is at moment `tau` of the films' timeline: its own time, whether it holds
 * there with no footage, before its stretch of a section starts or after it ends, and whether
 * that stretch is long enough to show (`QUIET_GAP`). Without a timeline both films play from
 * their start. */
export function filmAt(timeline: Timeline, side: "before" | "after", tau: number): { t: number; held: boolean; shown: boolean } {
  if (!timeline?.length) return { t: tau, held: false, shown: false };
  let section = timeline[0]!;
  for (const s of timeline) if (s.at <= tau) section = s;
  const f = footage(section, side);
  const into = tau - section.at - f.delay;
  const plays = f.to - f.from;
  if (into < -1e-6) return { t: f.from, held: true, shown: f.delay >= QUIET_GAP };
  if (into > plays + 1e-6) return { t: f.to, held: true, shown: section.length - f.delay - plays >= QUIET_GAP };
  return { t: f.from + Math.max(0, into), held: false, shown: false };
}

/** The moment of the films' timeline where one film plays its time `t`. */
export function timelineAt(timeline: Timeline, side: "before" | "after", t: number): number {
  if (!timeline?.length) return t;
  const section = timeline.find((s) => t < footage(s, side).to) ?? timeline[timeline.length - 1]!;
  const f = footage(section, side);
  return section.at + f.delay + Math.max(0, Math.min(t, f.to) - f.from);
}

/** The stretches of the films' timeline where one film has no footage long enough to show
 * (`QUIET_GAP`); a shorter one holds its frame quietly. */
export function gaps(timeline: Timeline, side: "before" | "after"): { start: number; stop: number }[] {
  const out: { start: number; stop: number }[] = [];
  for (const s of timeline ?? []) {
    const f = footage(s, side);
    const plays = s.at + f.delay + (f.to - f.from);
    if (f.delay >= QUIET_GAP) out.push({ start: s.at, stop: s.at + f.delay });
    if (s.at + s.length - plays >= QUIET_GAP) out.push({ start: plays, stop: s.at + s.length });
  }
  return out;
}

/** Where a still differs, when both sides have one size to draw it on. */
export function stillMarks(still: Comparison): Box[] {
  const [bw, bh] = still.size_before;
  const [aw, ah] = still.size_after;
  return bw === aw && bh === ah ? still.moments.map((m) => m.box) : [];
}

/** Whether a picture is a film: by its kind, or by a comparison that is not of stills. */
export function isFilm(picture: Picture): boolean {
  return picture.kind === "film" || (picture.comparison != null && !picture.comparison.still);
}

/** Whether it is a text of a look, which shows as a diff rather than a picture. */
export const isText = (picture: Picture): boolean => picture.kind === "text";

/** A picture's name as a person reads it: its parts, without the extension. */
export function title(p: Parts): string {
  return [p.screen, p.viewport, p.theme].filter(Boolean).join(" / ");
}

/** The comparison in plain words. */
export function plainSummary(picture: Picture): string {
  const c = picture.comparison;
  if (!c) {
    if (picture.change === "new") return "Not in the base.";
    if (picture.change === "gone") return "In the base, not anymore.";
    return picture.summary;
  }
  const [bw, bh] = c.size_before;
  const [aw, ah] = c.size_after;
  switch (c.verdict) {
    case "same":
      return "Looks the same.";
    case "timing":
      return `Same pictures, different timing: ${c.total_after.toFixed(2)} s long, was ${c.total_before.toFixed(2)} s.`;
    case "changed":
      if (bw !== aw || bh !== ah) return `The size changed from ${bw}×${bh} to ${aw}×${ah}.`;
      if (c.still) return `${picture.summary}.`;
      if (!c.moments.length) return "The steps it marked changed.";
      return `Changes at ${c.moments.map((m) => `${m.after[0].toFixed(2)} s`).join(", ")}${c.marks_changed ? ", and the steps it marked changed" : ""}.`;
  }
}

/** A film's moment as the page shows and keeps it: to the hundredth of a second, rounded once,
 * so the clock a person reads and the moment a note keeps are the same. */
export const hundredths = (t: number) => Math.round(t * 100) / 100;

/** The step of a film at `t` seconds, by what its check said happened in it: the newest shown
 * mark at or before `t`, or the span `t` is in when that started later. A free span after a
 * shown mark, such as the page settling after a press, is what the press did, so no step. */
export function stepAt(marks: Marks | null | undefined, t: number): string | undefined {
  let step: { name: string; at: number } | undefined;
  const shown = (marks?.marks ?? []).filter((m) => !m.hidden);
  for (const m of shown) if (m.at <= t && (!step || m.at >= step.at)) step = { name: m.name, at: m.at };
  for (const s of marks?.spans ?? []) {
    if (s.timing === "free" && shown.some((m) => m.at <= s.start)) continue;
    if (s.start <= t && t < s.end && (!step || s.start >= step.at)) step = { name: s.name, at: s.start };
  }
  return step?.name;
}
