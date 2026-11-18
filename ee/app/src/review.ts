// Review on the page: who looks at it and what they may do, where a picture stands, and what
// they can say (looks right, and an accept). Notes are in notes.ts.
// The host derives where review stands and words it for each role; the page shows that.

import type { PictureVerdict, Role, Run, Snapshot, Viewer } from "./proof";
import type { Shown } from "./pictures";
import { post, refusal } from "./api";
import { notesOn } from "./notes";
import { short } from "./short";

/** Who the page is shown to: a person as the host says (`Viewer`), or anyone through a share
 * link, which only views. */
export type Looker = Viewer | "shared";

/** The signed in person looking, if any. */
export const signedIn = (looker: Looker | undefined) => (typeof looker === "object" && looker.login !== null ? looker : undefined);

/** Whether the page only views, through a share link: nothing to press, no one asked to sign in. */
export const onlyViews = (looker: Looker | undefined) => looker === "shared";

/** Whether to offer `looker` a way to sign in: someone signed out, not through a share link. */
export const maySignIn = (looker: Looker | undefined) => typeof looker === "object" && looker.login === null;

/** What `looker` may do on the repository: read, also for someone signed out or through a share link. */
export const roleOf = (looker: Looker | undefined): Role => signedIn(looker)?.role ?? "read";

/** Whether `looker` may give verdicts (write access) or accept weakenings (admin rights). */
export const canSay = (looker: Looker | undefined) => roleOf(looker) !== "read";
export const canAccept = (looker: Looker | undefined) => roleOf(looker) === "admin";

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
 * looks right verdict on its content; without either it waits for a look (`none`). The host's
 * review says the same of the newest head's pictures, but it reaches the page after the verdict
 * or note that moved it, so the page reads the verdicts and notes it already holds. */
export function standingOf(state: Snapshot, run: Run, p: Shown): "needs_change" | "looks_right" | "none" {
  if (notesOn(state, p.check, p.picture.name, run).length) return "needs_change";
  return verdictOf(state, run, p) ? "looks_right" : "none";
}

/** Where a verdict came from, beside who gave it: an earlier push, or an earlier run. */
export function carried(verdict: PictureVerdict, run: Run): string {
  if (verdict.head !== run.head) return `carried over from an earlier push (${short(verdict.head)})`;
  if (verdict.run !== run.id) return "carried over from an earlier run";
  return "";
}

/** Sends what a person says about run `run` (`verdicts` or `accept`). The error the host gave,
 * if it refused. */
const say = async (run: string, what: "verdicts" | "accept", body: object) =>
  refusal(await post(`/api/v2/runs/${encodeURIComponent(run)}/${what}`, body));

export const verdict = (run: string, p: Shown) => say(run, "verdicts", { check: p.check, picture: p.picture.name });

export const accept = (run: string) => say(run, "accept", {});

/** Whether `value` is a viewer as the host answers it. */
export function isViewer(value: unknown): value is Viewer {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  if (v.login === null) return true;
  return typeof v.login === "string" && (v.role === "read" || v.role === "write" || v.role === "admin");
}

/** Who looks at the page of a work of `repository`, once the host says. */
export async function viewerOf(repository: string): Promise<Viewer> {
  try {
    const answer = await fetch(`/api/v2/viewer/${repository}`, { credentials: "same-origin" });
    const body: unknown = answer.ok ? await answer.json() : undefined;
    if (isViewer(body)) return body;
  } catch {
    // Without an answer the page offers only signing in.
  }
  return { login: null };
}

/** Where signing in starts, coming back to this page. */
export const signIn = () => `/login?next=${encodeURIComponent(location.pathname)}`;
