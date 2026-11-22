// A stand in for the proof host, so a page shows a state it is told: the live connection sends
// one message and stays open, the API answers who looks, the grant points at files this file
// serves, and the clock stands still, so every still of a state comes out the same.

import type { Page } from "@playwright/test";
import type { Listing, Live, Viewer } from "../src/proof";

/** The moment every page is shown at, in seconds since the epoch. */
export const NOW = 1_795_000_000;

/** A run's files by their path in its proof bundle. */
export type Files = Record<string, { type: string; body: string | Buffer }>;

/** What a page is shown: the first message of its live connection (none keeps it connecting),
 * or a close that says the page is gone; who looks; and the files a run's pictures load from:
 * `files` for every run, or a run's own under `runFiles`, granted in a folder of its own. */
export type World = {
  live?: Live | Listing;
  gone?: string;
  viewer?: Viewer;
  files?: Files;
  runFiles?: Record<string, Files>;
};

/** The page's live connection, to send it more as the host would. */
export type Socket = { send: (message: unknown) => void };

/** Opens `path` against a host that answers as `world` says, and waits until it settles. */
export async function show(page: Page, path: string, world: World): Promise<void> {
  await serve(page, world);
  await page.goto(path);
  await settled(page, !!world.files);
}

/** Answers `page` as a host that `world` describes, before it opens anything: the live
 * connection sends the world's first message, then whatever is sent through what it returns. */
export async function serve(page: Page, world: World): Promise<Socket> {
  let open: { send: (text: string) => void } | undefined;
  const waiting: string[] = [];
  await page.clock.setFixedTime(NOW * 1000);
  await page.routeWebSocket(/\/live\//, (socket) => {
    if (world.gone) {
      socket.close({ code: 1008, reason: world.gone });
      return;
    }
    if (world.live) socket.send(JSON.stringify(world.live));
    open = socket;
    for (const text of waiting.splice(0)) socket.send(text);
  });
  await page.route("**/api/v2/**", (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.startsWith("/api/v2/viewer/")) return route.fulfill({ json: world.viewer ?? { login: null } });
    if (url.pathname.endsWith("/grant")) {
      const run = decodeURIComponent(url.pathname.split("/").at(-2) ?? "");
      return route.fulfill({ json: { files: world.runFiles?.[run] ? `/files/${run}/` : "/files/", expires: NOW + 3600 } });
    }
    if (url.pathname === "/api/v2/shares") return route.fulfill({ json: { shares: [] } });
    // A verdict is taken; the page then waits for the live event that carries it.
    if (url.pathname.endsWith("/verdicts") || url.pathname.endsWith("/notes")) return route.fulfill({ json: {} });
    return route.fulfill({ status: 404, json: { error: "not in this world" } });
  });
  await page.route("**/files/**", (route) => {
    const path = decodeURIComponent(new URL(route.request().url()).pathname.slice("/files/".length));
    const [run, ...rest] = path.split("/");
    const file = world.runFiles?.[run!]?.[rest.join("/")] ?? world.files?.[path];
    if (!file) return route.fulfill({ status: 404 });
    // A range, as the files origin answers it (ee/host/src/files.rs): a browser seeks a film
    // only in one whose origin does.
    const body = typeof file.body === "string" ? Buffer.from(file.body) : file.body;
    const range = /^bytes=(\d+)-(\d*)$/.exec(route.request().headers()["range"] ?? "");
    if (!range) return route.fulfill({ contentType: file.type, body, headers: { "Accept-Ranges": "bytes" } });
    const from = Number(range[1]);
    const to = Math.min(range[2] ? Number(range[2]) : body.length - 1, body.length - 1);
    return route.fulfill({
      status: 206,
      contentType: file.type,
      body: body.subarray(from, to + 1),
      headers: { "Accept-Ranges": "bytes", "Content-Range": `bytes ${from}-${to}/${body.length}` },
    });
  });
  return {
    send: (message) => {
      const text = JSON.stringify(message);
      if (open) open.send(text);
      else waiting.push(text);
    },
  };
}

/** Waits until the page drew what it was told: its fonts, and, for a run with pictures, every
 * picture it shows loaded from the files. */
export async function settled(page: Page, pictures: boolean): Promise<void> {
  await page.locator("main > *").first().waitFor();
  if (pictures) await page.locator('img[src^="/files/"]').first().waitFor();
  // A lazy thumbnail below the fold would never load, yet a still of the whole page shows it.
  await page.waitForFunction(() =>
    [...document.images].every((i) => {
      i.loading = "eager";
      return i.complete;
    }),
  );
  await page.evaluate(() => document.fonts.ready);
}
