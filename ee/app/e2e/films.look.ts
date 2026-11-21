// The host's pages in motion, as films beside the stills: a run arriving live, comparing and
// reviewing with the keys, a note made again on another variant and edited on all of them, full
// screen and zoom, a picture to look at again, a failed run, weaker proof, a new film, sharing.
// Fearlezz compares each film with the same test on the base commit (stable states, time warped), so
// a change to how the pages move shows on the pull request as a changed film.
//
// A film is `<project>/work/<name>/<theme>.webm`, such as `desktop/work/live-run/light.webm`,
// which the proof host groups by screen, viewport and theme, as it does stills.

import { type Page, type TestInfo, test } from "@playwright/test";
import type { Note, PictureVerdict, Snapshot } from "../src/proof";
import { NOW, type Socket, type World, serve, settled } from "./host";
import { WORK, WORK_PATH } from "./states";

// Every test here is recorded, at the viewport scaled to fit 800 pixels, its proportions kept.
test.use({ video: "on" });

const pause = (page: Page, ms: number) => page.waitForTimeout(ms);
const snapshotOf = (name: string) => WORK[name]!.world.live as Snapshot & { kind: "snapshot" };

/** Keeps the film of `page` as `work/<name>/<theme>` of the test's project, in the theme the
 * page was shown in. */
async function keep(page: Page, info: TestInfo, name: string) {
  const theme = (await page.evaluate(() => matchMedia("(prefers-color-scheme: dark)").matches)) ? "dark" : "light";
  await page.close();
  const out = process.env.FEARLEZZ_OUT ?? "e2e";
  await page.video()?.saveAs(`${out}/films/${info.project.name}/work/${name}/${theme}.webm`);
}

/** Scrolls the picture under review to the top of the screen, as a person would before looking
 * at it, so the film shows what it is about rather than the page above it. */
async function toStage(page: Page) {
  await page.locator("[data-stage]").evaluate((el) => scrollTo({ top: el.getBoundingClientRect().top + scrollY - 16, behavior: "smooth" }));
  await pause(page, 900);
}

/** Answers verdicts and notes as the host does: taken, then streamed to the page. */
async function answers(page: Page, socket: Socket, seq: { n: number }) {
  await page.route("**/api/v2/runs/*/verdicts", async (route) => {
    const ask = route.request().postDataJSON() as { check: string; picture: string };
    const run = snapshotOf("waits-for-you").runs[0]!;
    const key = run.checks.find((c) => c.id === ask.check)?.keys?.[ask.picture] ?? "";
    const verdict: PictureVerdict = { ...ask, key, by: "mara", at: NOW, run: run.id, head: run.head };
    await route.fulfill({ status: 201, json: { verdict } });
    socket.send({ kind: "work", seq: ++seq.n, change: { kind: "verdict", verdict } });
  });
  await page.route("**/api/v2/runs/*/notes", async (route) => {
    const ask = route.request().postDataJSON() as Omit<Note, "id" | "run" | "head" | "by" | "at" | "state">;
    const run = snapshotOf("waits-for-you").runs[0]!;
    const note: Note = { ...ask, id: `n${++seq.n}`, run: run.id, head: run.head, by: "mara", at: NOW, state: "open" };
    await route.fulfill({ status: 201, json: { note } });
    socket.send({ kind: "work", seq: ++seq.n, change: { kind: "note", note } });
  });
}

test("film: a run arrives live", async ({ page }, info) => {
  const running = snapshotOf("running");
  const final = snapshotOf("waits-for-you");
  const done = final.runs.find((r) => r.id === running.runs[0]!.id)!;
  // The page opens connecting; the run is sent as it goes.
  const { live: _first, ...world } = WORK["waits-for-you"]!.world;
  const socket = await serve(page, world);
  await page.goto(WORK_PATH);
  await pause(page, 1200);
  let seq = running.seq;
  socket.send({ ...running, runs: [running.runs[0]] });
  await pause(page, 1000);
  const id = running.runs[0]!.id;
  for (const check of done.checks) {
    const was = running.runs[0]!.checks.find((c) => c.id === check.id);
    if (was?.entry) continue;
    if (was?.started == null) socket.send({ kind: "event", seq: ++seq, run: id, change: { kind: "started", check: check.id, at: NOW } });
    await pause(page, 220);
    socket.send({ kind: "event", seq: ++seq, run: id, change: { kind: "finished", check: check.entry } });
    await pause(page, 200);
  }
  socket.send({
    kind: "event",
    seq: ++seq,
    run: id,
    change: {
      kind: "done",
      checks: done.checks.map((c) => c.entry),
      looks: done.checks.flatMap((c) => (c.look ? [c.look] : [])),
      keys: Object.fromEntries(done.checks.filter((c) => c.keys).map((c) => [c.id, c.keys])),
    },
  });
  socket.send({ kind: "work", seq: ++seq, change: { kind: "review", review: final.review } });
  await settled(page, true);
  await pause(page, 1500);
  await keep(page, info, "live-run");
});

/** Opens the work whose pictures wait, answering verdicts and notes as the host does. */
async function waiting(page: Page) {
  const socket = await serve(page, WORK["waits-for-you"]!.world);
  await answers(page, socket, { n: snapshotOf("waits-for-you").seq + 100 });
  await page.goto(WORK_PATH);
  await settled(page, true);
  await pause(page, 600);
  await toStage(page);
}

// Each film stays short, so its script and the recording fit one test's time.
test("film: comparing with the keys", async ({ page }, info) => {
  await waiting(page);
  for (const key of ["2", "3", "4", "5", "1", "t", "v"]) {
    await page.keyboard.press(key);
    await pause(page, 800);
  }
  await keep(page, info, "comparing");
});

test("film: reviewing with the keys", async ({ page }, info) => {
  await waiting(page);
  for (const key of ["y", "y", "y", "?"]) {
    await page.keyboard.press(key);
    await pause(page, 900);
  }
  await keep(page, info, "reviewing");
});

test("film: a note, made again on the other theme", async ({ page }, info) => {
  await waiting(page);
  // Drag a region on the after, where the banner pushes the card down.
  const after = page.locator('[data-stage] [data-side="after"]').first();
  const box = (await after.boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.25, box.y + box.height * 0.12);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.75, box.y + box.height * 0.2, { steps: 12 });
  await page.mouse.up();
  await page.keyboard.type("The banner pushes the card down; show it as a toast.", { delay: 18 });
  await pause(page, 300);
  await page.keyboard.press("Control+Enter");
  await pause(page, 900);
  await page.keyboard.press("t");
  await pause(page, 900);
  await page.locator("[data-copy-note]").click();
  await pause(page, 1400);
  await keep(page, info, "note");
});

/** Opens `world`'s work page for a film, settled. */
/** A pointer the film shows, since a recording draws none: it follows the mouse and rings
 * while a button is down, so a person watching sees what was pressed and dragged. */
async function pointer(page: Page) {
  await page.addInitScript(() => {
    addEventListener("DOMContentLoaded", () => {
      const dot = document.createElement("div");
      dot.setAttribute("aria-hidden", "true");
      dot.style.cssText =
        "position:fixed;left:-9px;top:-9px;width:18px;height:18px;border-radius:50%;z-index:2147483647;pointer-events:none;" +
        "background:rgb(124 92 255 / 0.35);box-shadow:0 0 0 2px rgb(124 92 255 / 0.9);transition:scale 0.12s;translate:-40px -40px";
      document.body.append(dot);
      addEventListener("pointermove", (e: PointerEvent) => (dot.style.translate = `${e.clientX}px ${e.clientY}px`), true);
      addEventListener("pointerdown", () => (dot.style.scale = "0.7"), true);
      addEventListener("pointerup", () => (dot.style.scale = "1"), true);
    });
  });
}

/** Scrolls `target` into view as a person would, moves the pointer to it, and presses it. */
async function press(page: Page, target: import("@playwright/test").Locator) {
  await target.evaluate((el) => el.scrollIntoView({ behavior: "smooth", block: "center" }));
  await pause(page, 700);
  const box = (await target.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 18 });
  await pause(page, 200);
  await page.mouse.down();
  await pause(page, 90);
  await page.mouse.up();
}

async function open(page: Page, world: World) {
  await pointer(page);
  await serve(page, world);
  await page.goto(WORK_PATH);
  await settled(page, !!world.files);
  await pause(page, 600);
}

test("film: full screen, zoomed in on the change", async ({ page, isMobile }, info) => {
  await open(page, WORK["waits-for-you"]!.world);
  await page.keyboard.press("f");
  await pause(page, 700);
  if (isMobile) {
    for (const _ of [1, 2, 3]) {
      await page.keyboard.press("+");
      await pause(page, 500);
    }
    // Then moved with Space held up to the banner, and along it to read it.
    const after = (await page.locator('[data-stage] [data-side="after"]').boundingBox())!;
    const [x, y] = [after.x + after.width * 0.5, after.y + after.height * 0.5];
    await page.mouse.move(x, y, { steps: 10 });
    await page.keyboard.down(" ");
    await page.mouse.down();
    await page.mouse.move(x + after.width * 0.25, y + after.height * 0.2, { steps: 25 });
    await pause(page, 400);
    await page.mouse.move(x + after.width * 0.05, y + after.height * 0.2, { steps: 25 });
    await pause(page, 600);
    await page.mouse.up();
    await page.keyboard.up(" ");
  } else {
    // Pinched in on the banner, then moved along it with Space held.
    const after = (await page.locator('[data-stage] [data-side="after"]').boundingBox())!;
    await page.mouse.move(after.x + after.width * 0.3, after.y + after.height * 0.16);
    await page.keyboard.down("Control");
    for (const _ of [1, 2, 3, 4]) {
      await page.mouse.wheel(0, -80);
      await pause(page, 250);
    }
    await page.keyboard.up("Control");
    await pause(page, 500);
    await page.keyboard.down(" ");
    await page.mouse.down();
    await page.mouse.move(after.x + after.width * 0.1, after.y + after.height * 0.16, { steps: 20 });
    await page.mouse.up();
    await page.keyboard.up(" ");
  }
  await pause(page, 800);
  await page.keyboard.press("0");
  await pause(page, 600);
  await page.keyboard.press("Escape");
  await pause(page, 800);
  await keep(page, info, "full-screen");
});

test("film: a picture that changed since its look, looked at again", async ({ page }, info) => {
  const world = WORK["look-again"]!.world;
  const socket = await serve(page, world);
  await answers(page, socket, { n: snapshotOf("look-again").seq + 100 });
  await page.goto(WORK_PATH);
  await settled(page, true);
  await pause(page, 600);
  await toStage(page);
  await page.keyboard.press("t");
  await pause(page, 1200);
  await page.keyboard.press("y");
  await pause(page, 1400);
  await keep(page, info, "look-again");
});

test("film: a note edited on every note linked with it", async ({ page }, info) => {
  const world = WORK["needs-change"]!.world;
  const live = world.live as Snapshot & { kind: "snapshot" };
  const first = live.notes![0]!;
  await open(page, { ...world, live: { ...live, notes: [first, { ...first, id: "n2", picture: "profile/desktop/dark.png", link: first.id }] } });
  await page.route("**/api/v2/notes/*", (route) => route.fulfill({ json: { notes: [] } }));
  // The page opens on what waits for a look; the note is on the light theme.
  await press(page, page.locator('[data-picture="ee-page-look/profile/desktop/light.png"]'));
  await pause(page, 600);
  await press(page, page.locator('[data-notes] [data-note="n1"] [data-edit]'));
  await pause(page, 400);
  await page.locator("textarea[data-edit-words]").fill("");
  await page.keyboard.type("Keep the card where it was; show the banner as a toast.", { delay: 15 });
  await pause(page, 500);
  // Saved on every linked note, as the edit offers by default.
  await press(page, page.locator("[data-save-note]"));
  await pause(page, 1200);
  await keep(page, info, "edit-note");
});

test("film: a failed run, read to its cause", async ({ page }, info) => {
  await open(page, WORK.failed!.world);
  const output = page.locator("[data-failure] details.output summary").first();
  await output.scrollIntoViewIfNeeded();
  await pause(page, 700);
  await output.click();
  await pause(page, 1200);
  const fold = page.locator("[data-fold] summary").first();
  await fold.scrollIntoViewIfNeeded();
  await fold.click();
  await pause(page, 1200);
  await keep(page, info, "failed");
});

test("film: what the change weakens, opened", async ({ page }, info) => {
  await open(page, WORK["weaker-proof"]!.world);
  const show = page.locator("[data-show-change]").first();
  await show.scrollIntoViewIfNeeded();
  await pause(page, 600);
  await show.click();
  await pause(page, 1500);
  await page.locator("[data-accept]").first().hover();
  await pause(page, 900);
  await keep(page, info, "weaker-proof");
});

test("film: comparing by slider and onion skin", async ({ page }, info) => {
  await open(page, WORK["waits-for-you"]!.world);
  await toStage(page);
  await page.keyboard.press("2");
  await pause(page, 500);
  const frame = (await page.locator("[data-stage] .slide").boundingBox())!;
  const y = frame.y + frame.height / 2;
  await page.mouse.move(frame.x + frame.width * 0.5, y);
  await page.mouse.down();
  for (const at of [0.15, 0.85, 0.5]) await page.mouse.move(frame.x + frame.width * at, y, { steps: 25 });
  await page.mouse.up();
  await pause(page, 500);
  await page.keyboard.press("3");
  await pause(page, 500);
  const range = page.locator("[data-stage] .range input");
  for (const v of ["0", "100", "50"]) {
    await range.fill(v);
    await pause(page, 500);
  }
  await keep(page, info, "slider-onion");
});

test("film: a new film plays at a click, and a drag notes its frame", async ({ page }, info) => {
  await open(page, WORK["no-before"]!.world);
  await press(page, page.locator('[data-picture="ee-page-look/checkout/desktop.webm"]'));
  const layer = page.locator('[data-stage] [data-draw="after"]');
  await pause(page, 600);
  await toStage(page);
  await layer.click();
  await pause(page, 1200);
  // Dragged while it plays: it pauses on that frame, and the note marks it.
  const box = (await layer.boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.25, box.y + box.height * 0.3);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.55, { steps: 20 });
  await page.mouse.up();
  await page.locator("textarea[data-words]").pressSequentially("The total jumps here", { delay: 40 });
  await pause(page, 800);
  await keep(page, info, "new-film");
});

test("film: sharing the work", async ({ page }, info) => {
  await open(page, WORK["waits-for-you"]!.world);
  await page.locator("[data-share]").click();
  await pause(page, 1500);
  await keep(page, info, "share");
});
