// What a repository page knows: every work of the repository, newest activity first, from the
// listing its live connection sends first, with each work replaced as it changes. The host
// words where each stands and what waits on it.

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
