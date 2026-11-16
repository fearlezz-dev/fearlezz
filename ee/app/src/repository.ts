// What a repository page knows: every work of the repository, newest activity first, from the
// listing its live connection sends first, with each work replaced as it changes.

import type { Listed, Listing } from "./proof";

/** The works of a repository as its page shows them. */
export type Works = { repository: string; works: Listed[] };

/** The works with `listed` in place of the same work, newest activity first; the same works
 * when they hold that work's change already (by its sequence number). */
export function upsert(state: Works, listed: Listed): Works {
  const old = state.works.find((w) => w.path === listed.path);
  if (old && old.seq >= listed.seq) return state;
  const works = [listed, ...state.works.filter((w) => w.path !== listed.path)];
  works.sort((a, b) => b.active - a.active);
  return { ...state, works };
}

/** Takes the messages of one live connection in order, keeping works that arrive before the
 * listing until it comes. */
export class ListingFeed {
  private state: Works | undefined;
  private early: Listed[] = [];

  take(message: Listing): Works | undefined {
    if (message.kind === "snapshot") {
      this.state = this.early.reduce(upsert, { repository: message.repository, works: message.works });
      this.early = [];
    } else {
      const { kind: _, ...listed } = message;
      if (this.state) this.state = upsert(this.state, listed);
      else this.early.push(listed);
    }
    return this.state;
  }
}

/** Where a work's newest run stands, in one word. */
export type Standing = "none" | "running" | "passed" | "waits" | "failed" | "cancelled";

export function standingOf(listed: Listed): Standing {
  const latest = listed.latest;
  if (!latest) return "none";
  if (latest.state === "running") return "running";
  if (latest.state === "cancelled") return "cancelled";
  if (latest.failed > 0) return "failed";
  return waitsOn(listed).length ? "waits" : "passed";
}

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** What waits on a work, each as a short phrase: pictures without a verdict, open notes, and a
 * weakening for an admin. */
export function waitsOn(listed: Listed): string[] {
  const waits: string[] = [];
  if (listed.pictures) waits.push(`${count(listed.pictures, "picture", "pictures")} to look at`);
  if (listed.notes) waits.push(count(listed.notes, "open note", "open notes"));
  if (listed.weakened) waits.push("a weakening to accept");
  return waits;
}
