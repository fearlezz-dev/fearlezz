// Review on the page: who looks at it and what they may do, where a picture stands (an open
// note on it says it needs change, else a verdict on its content says it looks right), what
// waits for this person, and what they can say (looks right, and an accept). Notes are in
// notes.ts.
// The host derives where review stands; the page only words it for the person looking.

import type { PictureVerdict, Review, Run, Snapshot } from "./proof";
import type { Shown } from "./pictures";
import { post, refusal } from "./api";
import { notesOn } from "./notes";

/** Who the page is shown to: nobody signed in (or anyone, through a share link, which only
 * views), or a login and what GitHub lets them do. */
export type Viewer = { login: null; shared?: boolean } | { login: string; role: "admin" | "write" | "read" };

/** Whether the page only views, through a share link: nothing to press, no one asked to sign in. */
export const onlyViews = (viewer: Viewer | undefined) => viewer?.login === null && !!viewer.shared;

/** Whether to offer `viewer` a way to sign in: someone signed out, not through a share link. */
export const maySignIn = (viewer: Viewer | undefined) => viewer?.login === null && !viewer.shared;

/** Whether `viewer` may give verdicts (write access) or accept weakenings (admin rights). */
export const canSay = (viewer: Viewer | undefined) => !!viewer?.login && viewer.role !== "read";
export const canAccept = (viewer: Viewer | undefined) => !!viewer?.login && viewer.role === "admin";

/** Who ran a local run, which is unproven and never reviewed; undefined for a CI run. */
export const localBy = (run: Run) => (run.source.kind === "local" ? run.source.login || "a developer" : undefined);

/** Whether a person looks at a picture: anything but the same, and never a flaky one. */
export const toLookAt = (p: Shown) => p.picture.change !== "same" && !p.picture.flaky;

/** What a picture of a run shows, as a verdict follows it; none until the host knows. */
export function keyOf(run: Run, p: Shown): string | undefined {
  return run.checks.find((c) => c.id === p.check)?.keys?.[p.picture.name];
}

/** The verdict on what a picture of a run shows, given on this run or any other. */
export function verdictOf(state: Snapshot, run: Run, p: Shown): PictureVerdict | undefined {
  const key = keyOf(run, p);
  if (!key) return undefined;
  return (state.verdicts ?? []).find((v) => v.check === p.check && v.picture === p.picture.name && v.key === key);
}

/** Where a picture of a run stands: an open note on it says it needs change, which outweighs a
 * looks right verdict on its content; without either it waits for a look (`none`). */
export function standingOf(state: Snapshot, run: Run, p: Shown): "needs_change" | "looks_right" | "none" {
  if (notesOn(state, p.check, p.picture.name, run).length) return "needs_change";
  return verdictOf(state, run, p) ? "looks_right" : "none";
}

/** Where a verdict came from, beside who gave it: an earlier push, or an earlier run. */
export function carried(verdict: PictureVerdict, run: Run): string {
  if (verdict.head !== run.head) return `carried over from an earlier push (${verdict.head.slice(0, 7)})`;
  if (verdict.run !== run.id) return "carried over from an earlier run";
  return "";
}

const pictures = (n: number) => `${n} ${n === 1 ? "picture" : "pictures"}`;

/** What waits, said to the person looking: what they can settle, and what waits for others. */
export function waitsFor(review: Review, viewer: Viewer | undefined): string {
  const head = review.head.slice(0, 7);
  const looks = review.pictures.filter((p) => p.standing === "waits").length;
  const weakened = review.weakened.length > 0 && !review.accept;
  const parts: string[] = [];
  if (looks) {
    const verb = looks === 1 ? "waits" : "wait";
    parts.push(canSay(viewer) ? `${pictures(looks)} ${verb} for your look` : `${pictures(looks)} ${verb} for a look`);
  }
  if (weakened) parts.push(canAccept(viewer) ? "a weakened proof waits for your accept" : "a weakened proof waits for an admin");
  const text = parts.join(", and ");
  return `${text.charAt(0).toUpperCase()}${text.slice(1)} at ${head}`;
}

/** Where review stands when open notes say pictures need change, in one line. */
export function standing(review: Review): string {
  const n = new Set((review.notes ?? []).map((note) => `${note.check}\n${note.picture}`)).size;
  return `${pictures(n)} ${n === 1 ? "needs" : "need"} change at ${review.head.slice(0, 7)}`;
}

/** Sends what a person says about run `run` (`verdicts` or `accept`). The error the host gave,
 * if it refused. */
const say = async (run: string, what: "verdicts" | "accept", body: object) =>
  refusal(await post(`/api/v2/runs/${encodeURIComponent(run)}/${what}`, body));

export const verdict = (run: string, p: Shown) => say(run, "verdicts", { check: p.check, picture: p.picture.name });

export const accept = (run: string) => say(run, "accept", {});

/** Who looks at the page of a work of `repository`, once the host says. */
export async function viewerOf(repository: string): Promise<Viewer> {
  try {
    const answer = await fetch(`/api/v2/viewer/${repository}`, { credentials: "same-origin" });
    if (answer.ok) return (await answer.json()) as Viewer;
  } catch {
    // Without an answer the page offers only signing in.
  }
  return { login: null };
}

/** Where signing in starts, coming back to this page. */
export const signIn = () => `/login?next=${encodeURIComponent(location.pathname)}`;
