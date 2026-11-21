// Pictures fetched and decoded before they show: the next ones a person steps to, so a step
// swaps them in at once instead of waiting on the files origin, which may be far away. The
// page keeps each decoded image, as the browser answers a picture it holds without asking again.
// A film is fetched whole into the browser's cache instead, which its player then reads after
// the files origin says it has not changed (its ETag), so it plays at once too.

/** How many decoded pictures the page keeps, the newest asked for last. */
const KEPT = 24;

const kept = new Map<string, { image: HTMLImageElement; decoded: Promise<void> }>();

/** The picture at `url`, fetched and decoded once; ready when it is (or could not be). */
export function ready(url: string): Promise<void> {
  const held = kept.get(url);
  if (held) {
    // Asked again, it is the newest.
    kept.delete(url);
    kept.set(url, held);
    return held.decoded;
  }
  const image = new Image();
  image.decoding = "async";
  image.src = url;
  const decoded = image.decode().catch(() => {});
  kept.set(url, { image, decoded });
  for (const old of kept.keys()) {
    if (kept.size <= KEPT) break;
    kept.delete(old);
  }
  return decoded;
}

/** Fetches and decodes the pictures at `urls` ahead, in order. */
export function preload(urls: readonly string[]): void {
  for (const url of urls) if (url) void ready(url);
}

/** How many films are fetched ahead at once: they are large, and each waits its turn. */
const AT_ONCE = 2;

const fetched = new Set<string>();
const queued: string[] = [];
let fetching = 0;

function next(): void {
  while (fetching < AT_ONCE && queued.length) {
    const url = queued.shift()!;
    fetching += 1;
    void fetch(url, { mode: "cors", credentials: "omit" })
      .then((answer) => answer.arrayBuffer())
      .catch(() => fetched.delete(url))
      .finally(() => {
        fetching -= 1;
        next();
      });
  }
}

/** Fetches the films at `urls` ahead, whole, in order, a few at a time. */
export function prefetch(urls: readonly string[]): void {
  for (const url of urls) {
    if (!url || fetched.has(url)) continue;
    fetched.add(url);
    queued.push(url);
  }
  next();
}
