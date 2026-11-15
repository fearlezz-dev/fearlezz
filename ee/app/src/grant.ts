// Where a run's pictures and films load from: the files origin, under a grant the app origin
// hands out to someone who may see the run, which lasts an hour. The page asks again a few
// minutes before it ends, so pictures keep loading on a page left open.

import { type Accessor, createEffect, createSignal, onCleanup } from "solid-js";

/** What the host answers for a grant: the run's folder on the files origin, and when the grant
 * ends, in seconds since the epoch. */
type Granted = { files: string; expires: number };

/** How long before a grant ends the page asks for the next. */
const EARLY_SECONDS = 300;

/** The URL of the file at `path` (a path in the proof bundle) under the folder `files`. */
export function fileUrl(files: string, path: string): string {
  return files + path.split("/").map(encodeURIComponent).join("/");
}

/** The folder of the files of run `run()` on the files origin, once granted; empty until then
 * and when the host refuses (the page then shows the pictures as not loaded). */
export function grantOf(run: Accessor<string>): Accessor<string> {
  const [files, setFiles] = createSignal("");
  createEffect(() => {
    const id = run();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    const ask = async (wait: number) => {
      try {
        const answer = await fetch(`/api/v2/runs/${encodeURIComponent(id)}/grant`, { credentials: "same-origin" });
        if (!answer.ok) throw new Error(`the host answered ${answer.status}`);
        const granted = (await answer.json()) as Granted;
        if (stopped) return;
        setFiles(granted.files);
        const left = granted.expires - Date.now() / 1000 - EARLY_SECONDS;
        timer = setTimeout(() => ask(1000), Math.max(left, 30) * 1000);
      } catch {
        // A host that cannot answer now is asked again, less often each time.
        if (!stopped) timer = setTimeout(() => ask(Math.min(wait * 2, 60_000)), wait);
      }
    };
    setFiles("");
    void ask(1000);
    onCleanup(() => {
      stopped = true;
      clearTimeout(timer);
    });
  });
  return files;
}
