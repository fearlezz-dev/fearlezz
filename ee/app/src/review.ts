// Review on the page: who looks at it and what they may do, where a picture stands, and what
// they can say (looks right, and an accept). Notes are in notes.ts.
// The host derives where review stands and words it for each role; the page shows that.

import type { PictureVerdict, Role, Run, Snapshot, Viewer } from "./proof";
import { type Shown, isFilm, picturesOf } from "./pictures";
import { post, refusal, send } from "./api";
import { notesOn } from "./notes";
import { hoping, verdictChange } from "./hopes";
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

/** How far one looks right reaches among a screen's variants: the picture alone, every
 * variant of its viewport, every variant of its theme, or the whole screen. */
export const SCOPES = ["this", "viewport", "theme", "screen"] as const;
export type Scope = (typeof SCOPES)[number];

/** The pictures a looks right on `p` at `scope` says looks right, from its screen's `variants`:
 * those to look at that `scope` reaches, `p` first. A film shows what its still shows, so its
 * looks right reaches the still of its viewport and theme too. */
export function covered(p: Shown, variants: readonly Shown[], scope: Scope): Shown[] {
  const reaches = (q: Shown) => {
    switch (scope) {
      case "this":
        return isFilm(p.picture) && !isFilm(q.picture) && q.parts.viewport === p.parts.viewport && q.parts.theme === p.parts.theme;
      case "viewport":
        return q.parts.viewport === p.parts.viewport;
      case "theme":
        return q.parts.theme === p.parts.theme;
      case "screen":
        return true;
    }
  };
  return [p, ...variants.filter((q) => q.key !== p.key && toLookAt(q) && reaches(q))];
}

/** The run a work page opens on: the newest, or, when it is one leg of a CI run proved in legs,
 * the newest leg of that same run with pictures to look at, since only one leg has them. A
 * rerun is a run of its own, so it shows as it opens. */
export function opening(runs: Run[]): Run | undefined {
  const newest = runs[0];
  const legOf = (r: Run) =>
    r.source.kind === "ci" && newest?.source.kind === "ci" && r.head === newest.head && r.source.run_id === newest.source.run_id && r.source.run_attempt === newest.source.run_attempt;
  return runs.find((r) => legOf(r) && picturesOf(r).some(toLookAt)) ?? newest;
}

/** What a picture of a run shows, as a verdict follows it: its own file; none until the host
 * knows. */
export function keyOf(run: Run, p: Shown): string | undefined {
  return run.checks.find((c) => c.id === p.check)?.keys?.[p.picture.name];
}

/** Whether picture `p` of `run`, whose content is `key`, looks like the picture whose content
 * was `then`: the same file, or one the run's CLI compared it with and found the same. The
 * host's review uses the same rule. */
function alike(run: Run, p: Shown, key: string, then: string): boolean {
  return then === key || run.checks.find((c) => c.id === p.check)?.since?.[p.picture.name]?.[then] === true;
}

/** The newest verdict on a picture of a run that looks like it, given on this run or any other. */
export function verdictOf(state: Snapshot, run: Run, p: Shown): PictureVerdict | undefined {
  const key = keyOf(run, p);
  if (!key) return undefined;
  return (state.verdicts ?? [])
    .filter((v) => v.check === p.check && v.picture === p.picture.name && alike(run, p, key, v.key))
    .sort((a, b) => b.at - a.at)[0];
}

/** Where a picture of a run stands: an open note on it says it needs change, which outweighs a
 * looks right verdict on its content; without either it waits for a look (`none`). The host's
 * review says the same of the newest head's pictures, but it reaches the page after the verdict
 * or note that moved it, so the page reads the verdicts and notes it already holds. */
export function standingOf(state: Snapshot, run: Run, p: Shown): "needs_change" | "looks_right" | "none" {
  if (notesOn(state, p.check, p.picture.name, run).length) return "needs_change";
  return verdictOf(state, run, p) ? "looks_right" : "none";
}

/** What a person said about what a picture showed before it changed: the newest looks right
 * on a picture that does not look like it (unless one does), and the ids of its open notes made
 * on a run whose picture does not look like it. The host's review says the same as
 * `changed_since`; the page reads what it holds, so a new verdict clears it at once. A note
 * whose run the page does not hold says nothing. */
export type Since = { verdict: PictureVerdict | undefined; notes: string[] };
export function changedSince(state: Snapshot, run: Run, p: Shown): Since | undefined {
  const key = keyOf(run, p);
  if (!key) return undefined;
  const verdict = verdictOf(state, run, p)
    ? undefined
    : (state.verdicts ?? []).filter((v) => v.check === p.check && v.picture === p.picture.name).sort((a, b) => b.at - a.at)[0];
  const then = (id: string) => {
    const made = state.runs.find((r) => r.id === id);
    return made && keyOf(made, p);
  };
  const notes = notesOn(state, p.check, p.picture.name, run)
    .filter((n) => {
      const was = then(n.note.run);
      return was !== undefined && !alike(run, p, key, was);
    })
    .map((n) => n.note.id);
  return verdict || notes.length ? { verdict, notes } : undefined;
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

/** Says picture `p` of run `run` looks right, as `by`, shown at once with the picture's open
 * notes resolved. The error the host gave, if it refused. */
export function verdict(state: Snapshot, run: Run, p: Shown, by: string): Promise<string | undefined> {
  const ask = () => post(`/api/v2/runs/${encodeURIComponent(run.id)}/verdicts`, { check: p.check, picture: p.picture.name });
  const key = keyOf(run, p);
  if (!key) return ask().then(refusal);
  const open = notesOn(state, p.check, p.picture.name, run).map((n) => n.note);
  const given = { check: p.check, picture: p.picture.name, key, by, at: Date.now() / 1000, run: run.id, head: run.head };
  return hoping(verdictChange(given, open), ask);
}

/** Takes back the verdict on a picture's content, shown at once; the error the host gave, if it
 * refused. */
export function withdraw(run: Run, p: Shown): Promise<string | undefined> {
  const ask = () => send("DELETE", `/api/v2/runs/${encodeURIComponent(run.id)}/verdicts`, { check: p.check, picture: p.picture.name });
  const key = keyOf(run, p);
  if (!key) return ask().then(refusal);
  return hoping({ kind: "withdrawn", check: p.check, picture: p.picture.name, key }, ask);
}

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
