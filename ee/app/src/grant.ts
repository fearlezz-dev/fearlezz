// Where a run's pictures and films load from: the files origin, under a grant the app origin
// hands out to someone who may see the run, which lasts an hour. The page asks again a few
// minutes before it ends, so pictures keep loading on a page left open.

import { type Accessor, createEffect, createSignal, onCleanup } from "solid-js";
import { sharedBy } from "./place";

/** What the host answers for a grant: the run's folder on the files origin, and when the grant
 * ends, in seconds since the epoch. */
type Granted = { files: string; expires: number };

/** How long before a grant ends the page asks for the next. */
const EARLY_SECONDS = 300;

/** The URL of the file at `path` (a path in the proof bundle) under the folder `files`. */
export function fileUrl(files: string, path: string): string {
  return files + path.split("/").map(encodeURIComponent).join("/");
}

/** Where the page asks for a grant to the files of run `run`: through a share link, with its
 * token, which the host takes for the runs of the link's work alone. */
export function grantPath(run: string, share: string | undefined): string {
  const path = `/api/v2/runs/${encodeURIComponent(run)}/grant`;
  return share ? `${path}?share=${encodeURIComponent(share)}` : path;
}

// Grants by run, shared by every part of the page that loads a run's files, until shortly
// before each ends.
const grants = new Map<string, { asking: Promise<Granted>; until: number }>();

/** A grant to the files of run `run`, asked once and again shortly before it ends. */
export function granted(run: string): Promise<Granted> {
  const kept = grants.get(run);
  if (kept && kept.until > Date.now() / 1000) return kept.asking;
  const entry = {
    asking: (async () => {
      const answer = await fetch(grantPath(run, sharedBy(location.pathname)), { credentials: "same-origin" });
      if (!answer.ok) throw new Error(`the host answered ${answer.status}`);
      return (await answer.json()) as Granted;
    })(),
    until: Number.POSITIVE_INFINITY,
  };
  grants.set(run, entry);
  entry.asking.then(
    (g) => {
      entry.until = g.expires - EARLY_SECONDS;
    },
    () => grants.delete(run),
  );
  return entry.asking;
}

/** The folder of the files of run `run()` on the files origin, once granted; empty until then
 * and when the host refuses (the page then shows the pictures as not loaded). A grant is only
 * ever given out for the run it was asked for, also while the run changes. */
export function grantOf(run: Accessor<string>): Accessor<string> {
  const [held, setHeld] = createSignal({ run: "", files: "" });
  createEffect(() => {
    const id = run();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    const ask = async (wait: number) => {
      try {
        const grant = await granted(id);
        if (stopped) return;
        setHeld({ run: id, files: grant.files });
        const left = grant.expires - Date.now() / 1000 - EARLY_SECONDS;
        timer = setTimeout(() => ask(1000), Math.max(left, 30) * 1000);
      } catch {
        // A host that cannot answer now is asked again, less often each time.
        if (!stopped) timer = setTimeout(() => ask(Math.min(wait * 2, 60_000)), wait);
      }
    };
    void ask(1000);
    onCleanup(() => {
      stopped = true;
      clearTimeout(timer);
    });
  });
  return () => (held().run === run() ? held().files : "");
}
