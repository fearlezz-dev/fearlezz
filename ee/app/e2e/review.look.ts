// How a person moves through the pictures while reviewing: a verdict moves on to the next
// picture that waits, and stepping keeps the picture where it was on the screen.

import { expect, test } from "@playwright/test";
import { show } from "./host";
import { WORK, WORK_PATH } from "./states";

const current = "[data-picture][aria-current=true]";

test("a verdict moves on to the next picture that waits", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  const first = await page.locator(current).getAttribute("data-picture");
  await page.locator("[data-say=looks_right]").click();
  await expect(page.locator(current)).not.toHaveAttribute("data-picture", first!);
});

// The host may be on the far side of the world: what a person says shows before it answers.
test("looks right shows at once, and a refusal takes it back", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  let answer: (refuse: boolean) => Promise<void> = async () => {};
  await page.route("**/verdicts", (route) => {
    answer = (refuse) => route.fulfill(refuse ? { status: 403, json: { error: "no write access" } } : { json: {} });
  });
  const first = await page.locator(current).getAttribute("data-picture");
  const tile = page.locator(`[data-picture="${first}"]`);
  await page.keyboard.press("y");
  await expect(tile).toHaveAttribute("data-verdict", "looks_right");
  await expect(page.locator(current)).not.toHaveAttribute("data-picture", first!);
  await expect.poll(() => answer(true).then(() => true)).toBe(true);
  await expect(tile).toHaveAttribute("data-verdict", "none");
  await expect(page.getByRole("alert")).toContainText("no write access");
});

// Most often a person goes through the pictures in order: the next ones are fetched and
// decoded before they step to them, so a step swaps the picture in at once.
test("the next picture is fetched before a person steps to it", async ({ page }) => {
  const asked: string[] = [];
  page.on("request", (r) => {
    if (r.url().includes("/files/")) asked.push(new URL(r.url()).pathname);
  });
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  const before = asked.length;
  await page.keyboard.press("j");
  const shown = page.locator('[data-stage] img[data-side="after"]');
  await expect.poll(async () => asked.indexOf(new URL((await shown.getAttribute("src"))!, page.url()).pathname)).toBeGreaterThanOrEqual(0);
  const at = asked.indexOf(new URL((await shown.getAttribute("src"))!, page.url()).pathname);
  expect(at, "asked for before the step").toBeLessThan(before);
});

// A screen's films come first; a film's looks right reaches the still of its viewport and
// theme, and a held Y, ⇧Y or ⌥Y reach further.
const swatch = (page: import("@playwright/test").Page, name: string) => page.locator(`.variant[data-picture="ee-page-look/profile/${name}"]`);

test("a film's looks right reaches the still of its viewport and theme", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  const screen = page.locator('[data-screen="ee-page-look/profile"] .variant');
  await expect(screen.first()).toHaveAttribute("data-picture", "ee-page-look/profile/desktop/light.webm");
  await swatch(page, "desktop/light.webm").click();
  await page.keyboard.press("y");
  await expect(swatch(page, "desktop/light.webm")).toHaveAttribute("data-verdict", "looks_right");
  await expect(swatch(page, "desktop/light.png")).toHaveAttribute("data-verdict", "looks_right");
  await expect(swatch(page, "desktop/dark.png")).toHaveAttribute("data-verdict", "none");
});

test("a held Y chooses how far looks right reaches", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await swatch(page, "desktop/dark.png").click();
  await page.keyboard.down("y");
  await expect(page.locator("[data-say-menu]")).toBeVisible();
  await page.keyboard.up("y");
  await expect(page.locator("[data-say-menu]")).toBeVisible();
  await expect(page.locator('[data-say-scope="viewport"]')).toContainText("All of desktop");
  await expect(page.locator('[data-say-scope="viewport"] .n')).toHaveText("4");
  await expect(page.locator('[data-say-scope="screen"] .n')).toHaveText("8");
  await page.keyboard.press("2");
  await expect(page.locator("[data-say-menu]")).toBeHidden();
  for (const name of ["desktop/light.webm", "desktop/dark.webm", "desktop/light.png", "desktop/dark.png"])
    await expect(swatch(page, name)).toHaveAttribute("data-verdict", "looks_right");
  await expect(swatch(page, "mobile/light.png")).toHaveAttribute("data-verdict", "none");
});

// Full screen often leaves Looks right below the window: a held Y floats the choice on the
// screen, it stays once Y is let go, and Esc closes it without leaving full screen.
// Notes are kept as fractions of the layer they are drawn on, so that layer must be the picture
// itself, at any window height: a picture no comparison measured too, which once letterboxed
// inside a frame wider than it, in full screen differently than on the page.
test("the layer notes are drawn on is the picture, on the page and in full screen", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('.variant[data-picture="ee-page-look/profile/desktop/light.png"]').click();
  const fits = async () => {
    const img = (await page.locator('[data-stage] img[data-side="after"]').boundingBox())!;
    const layer = (await page.locator('[data-stage] [data-draw="after"]').boundingBox())!;
    const natural = await page.locator('[data-stage] img[data-side="after"]').evaluate((i) => (i as HTMLImageElement).naturalWidth / (i as HTMLImageElement).naturalHeight);
    expect(Math.abs(layer.width - img.width), "as wide as the picture").toBeLessThan(1.5);
    expect(Math.abs(layer.height - img.height), "as tall as the picture").toBeLessThan(1.5);
    expect(Math.abs(img.width / img.height - natural), "the picture is not stretched").toBeLessThan(0.02);
  };
  await expect.poll(async () => (await page.locator("[data-stage] .frame").first().getAttribute("class"))?.includes("sized")).toBe(true);
  await fits();
  await page.keyboard.press("f");
  await page.waitForTimeout(300);
  await fits();
});

test("a note's words grow to six lines, then scroll inside", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  await page.keyboard.press("n");
  const words = page.locator("textarea[data-words]");
  const lines = (n: number) => Array.from({ length: n }, (_, k) => `line ${k + 1}`).join("\n");
  await words.fill(lines(1));
  const three = (await words.boundingBox())!.height;
  await words.fill(lines(5));
  const five = (await words.boundingBox())!.height;
  await words.fill(lines(12));
  const twelve = (await words.boundingBox())!.height;
  expect(five, "it grew with its words").toBeGreaterThan(three);
  expect(twelve - five, "it stops at six lines").toBeLessThan(five - three);
  expect(await words.evaluate((t) => t.scrollHeight > t.clientHeight), "and scrolls inside").toBe(true);
});

test("a held Y shows the choice on the screen, and Esc closes it", async ({ page }) => {
  await page.setViewportSize({ width: page.viewportSize()!.width, height: 420 });
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await swatch(page, "desktop/dark.png").click();
  await page.keyboard.press("f");
  await page.keyboard.down("y");
  const menu = page.locator("[data-say-menu]");
  await expect(menu).toBeVisible();
  await page.keyboard.up("y");
  await page.waitForTimeout(300);
  await expect(menu).toBeVisible();
  const box = (await menu.boundingBox())!;
  const size = page.viewportSize()!;
  expect(box.y, "inside the window").toBeGreaterThanOrEqual(0);
  expect(box.y + box.height, "inside the window").toBeLessThanOrEqual(size.height);
  expect(box.x + box.width).toBeLessThanOrEqual(size.width);
  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
  await expect(page.locator("[data-table]")).toHaveAttribute("data-full", "");
});

test("⌥Y says looks right on the viewport, ⇧Y on the whole screen", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await swatch(page, "mobile/light.png").click();
  await page.keyboard.press("Alt+KeyY");
  await expect(swatch(page, "mobile/dark.webm")).toHaveAttribute("data-verdict", "looks_right");
  await expect(swatch(page, "desktop/light.png")).toHaveAttribute("data-verdict", "none");
  await swatch(page, "desktop/light.png").click();
  await page.keyboard.press("Shift+Y");
  for (const name of ["desktop/light.webm", "desktop/dark.png", "mobile/light.png"])
    await expect(swatch(page, name)).toHaveAttribute("data-verdict", "looks_right");
});

test("the next films are fetched ahead too", async ({ page }) => {
  const ahead: string[] = [];
  page.on("request", (r) => {
    if (r.resourceType() === "fetch" && r.url().endsWith(".webm")) ahead.push(r.url());
  });
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('.variant[data-picture="ee-page-look/security/desktop/light.png"]').click();
  await expect.poll(() => ahead.length, "a film fetched whole before it shows").toBeGreaterThan(0);
});

test("stepping to the next picture keeps the page where it was", async ({ page }) => {
  // Scrolled down to the picture, the sheet of thumbnails out of sight above it.
  await page.setViewportSize({ width: page.viewportSize()!.width, height: 560 });
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  const before = await page.evaluate(() => window.scrollY);
  expect((await page.locator(current).boundingBox())!.y).toBeLessThan(0);
  for (const _ of [1, 2, 3]) {
    await page.keyboard.press("j");
    await page.waitForTimeout(150);
    expect(Math.abs((await page.evaluate(() => window.scrollY)) - before)).toBeLessThan(4);
  }
});

test("keys say looks right, take it back, and start a note", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  const first = await page.locator(current).getAttribute("data-picture");
  await page.keyboard.press("y");
  await expect(page.locator(current)).not.toHaveAttribute("data-picture", first!);
  await page.keyboard.press("n");
  await expect(page.locator("textarea[data-words]")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator("textarea[data-words]")).toHaveCount(0);
});

test("t and v switch to the screen's other theme and viewport", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/profile/desktop/light.png");
  await page.keyboard.press("t");
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/profile/desktop/dark.png");
  await page.keyboard.press("v");
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/profile/mobile/dark.png");
});

test("a verdict given shows Undo, which takes it back", async ({ page }) => {
  const world = WORK["needs-change"]!.world;
  await show(page, WORK_PATH, world);
  await page.locator('[data-picture="ee-page-look/profile/desktop/dark.png"]').click();
  let withdrawn = false;
  page.on("request", (r) => {
    if (r.method() === "DELETE" && r.url().endsWith("/verdicts")) withdrawn = true;
  });
  await page.keyboard.press("u");
  await expect.poll(() => withdrawn).toBe(true);
});

test("looks right pressed again takes it back", async ({ page }) => {
  await show(page, WORK_PATH, WORK["needs-change"]!.world);
  await page.locator('[data-picture="ee-page-look/profile/desktop/dark.png"]').click();
  const said = page.locator('[data-say="looks_right"]');
  await expect(said).toHaveAttribute("aria-pressed", "true");
  const withdrawn = page.waitForRequest((r) => r.method() === "DELETE" && r.url().endsWith("/verdicts"));
  await said.click();
  await withdrawn;
});

test("? shows the keys", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  await page.keyboard.press("?");
  await expect(page.locator("[data-keys]")).toBeVisible();
  await expect(page.locator("[data-keys]")).toContainText("Take back looks right");
});

test("a note of another variant is made here in its place and words, or lends its words", async ({ page }) => {
  await show(page, WORK_PATH, WORK["needs-change"]!.world);
  await page.locator('[data-picture="ee-page-look/profile/desktop/dark.png"]').click();
  const made = page.waitForRequest((r) => r.method() === "POST" && r.url().endsWith("/notes"));
  await page.locator("[data-copy-note]").click();
  const ask = (await made).postDataJSON();
  expect(ask).toMatchObject({ picture: "profile/desktop/dark.png", side: "after", region: { x: 0.25, y: 0.13, w: 0.5, h: 0.07 } });
  expect(ask.words).toContain("The banner pushes the card down");
  // On the phone the place differs: only the words, of either note (the one just made shows
  // at once).
  await page.locator('[data-picture="ee-page-look/profile/mobile/light.png"]').click();
  await expect(page.locator("[data-use-words]")).toHaveCount(2);
  await page.locator("[data-use-words]").first().click();
  await expect(page.locator("textarea[data-words]")).toHaveValue(/The banner pushes the card down/);
});

const zoomOf = (page: import("@playwright/test").Page) =>
  page.locator("[data-stage] [data-zoom-layer]").first().evaluate((el) => (el as HTMLElement).style.transform);

test("f shows the picture on the whole screen with its review, and Esc goes back", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  await page.keyboard.press("f");
  await expect(page.locator("[data-table][data-full]")).toBeVisible();
  await expect(page.locator("[data-table] [data-say=looks_right]")).toBeVisible();
  const table = (await page.locator("[data-table]").boundingBox())!;
  expect(table.width).toBe(page.viewportSize()!.width);
  expect(table.height).toBe(page.viewportSize()!.height);
  await page.keyboard.press("Escape");
  await expect(page.locator("[data-table][data-full]")).toHaveCount(0);
  await page.locator("[data-full-screen]").click();
  await expect(page.locator("[data-table][data-full]")).toBeVisible();
});

test("keys and buttons zoom both sides together, and 0 shows the whole picture", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  expect(await zoomOf(page)).toBe("none");
  await page.keyboard.press("+");
  await page.keyboard.press("+");
  const layers = page.locator("[data-stage] [data-zoom-layer]");
  await expect(layers).toHaveCount(2);
  const both = await layers.evaluateAll((els) => els.map((el) => (el as HTMLElement).style.transform));
  expect(both[0]).toContain("scale(2.25)");
  expect(both[1]).toBe(both[0]);
  await expect(page.locator("[data-zoom=fit]")).toHaveText("225%");
  await page.keyboard.press("0");
  expect(await zoomOf(page)).toBe("none");
  await page.locator("[data-zoom=in]").click();
  expect(await zoomOf(page)).toContain("scale(1.5)");
});

test("a pinch zooms where the pointer is, and Space with a drag moves the picture", async ({ page, isMobile }) => {
  // A phone pinches the page itself; its browser sends the page no wheel.
  test.skip(isMobile, "a phone has no wheel");
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  const after = page.locator('[data-stage] [data-side="after"]').first();
  const box = (await after.boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.2, box.y + box.height * 0.2);
  // A trackpad pinch reaches the page as a wheel with Control held.
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -200);
  await page.keyboard.up("Control");
  await expect.poll(() => zoomOf(page)).not.toBe("none");
  const zoomed = await zoomOf(page);
  await page.keyboard.down(" ");
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.1, box.y + box.height * 0.1, { steps: 5 });
  await page.mouse.up();
  await page.keyboard.up(" ");
  expect(await zoomOf(page)).not.toBe(zoomed);
  await expect(page.locator("[data-draft]")).toHaveCount(0);
});

test("the zoom stays on the screen's other theme", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  await page.keyboard.press("+");
  const zoomed = await zoomOf(page);
  await page.keyboard.press("t");
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/profile/desktop/dark.png");
  expect(await zoomOf(page)).toBe(zoomed);
});

test("the page opens on the first picture that waits for a look", async ({ page }) => {
  // The profile's light theme has a note waiting on the agent and its dark theme looks right.
  await show(page, WORK_PATH, WORK["needs-change"]!.world);
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/profile/mobile/light.png");
  // Opened, it stays while what waits changes.
  await page.keyboard.press("y");
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/profile/mobile/dark.png");
});

test("a picture or film that changed since its look or its note says to look again", async ({ page }) => {
  await show(page, WORK_PATH, WORK["look-again"]!.world);
  const profile = page.locator('[data-screen="ee-page-look/profile"]');
  await expect(profile.locator("[data-again-mark]")).toHaveText("Look again");
  await expect(profile.locator('[data-picture="ee-page-look/profile/desktop/dark.png"]')).toHaveAttribute("data-again", "");
  await expect(profile.locator('[data-picture="ee-page-look/profile/desktop/light.png"]')).toHaveAttribute("data-again", "");
  await expect(profile.locator('[data-picture="ee-page-look/profile/desktop/light.webm"]')).toHaveAttribute("data-again", "");
  await expect(profile.locator('[data-picture="ee-page-look/profile/mobile/light.png"]')).not.toHaveAttribute("data-again");
  // The page opens on the first picture to look at again: the film, as a screen's films come
  // first, which looked right before it changed.
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/profile/desktop/light.webm");
  await expect(page.locator(".verdict [data-again]")).toContainText("Changed since mara said it looks right at 2b7e91c");
  // The light theme's note was made before it changed: check it, then resolve or keep it.
  await profile.locator('[data-picture="ee-page-look/profile/desktop/light.png"]').click();
  await expect(page.locator(".verdict [data-again]")).toContainText("Changed since note 1");
  await expect(page.locator('[data-notes] [data-note="n1"] [data-changed-since]')).toBeVisible();
  // The dark theme looked right before it changed.
  await page.keyboard.press("t");
  await expect(page.locator(".verdict [data-again]")).toContainText("Changed since mara said it looks right at 2b7e91c");
  await expect(page.locator("[data-standing]")).toContainText("changed since a look");
});

test("a new film plays on its own, with no before", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('[data-picture="ee-page-look/checkout/desktop.webm"]').click();
  const film = page.locator('[data-stage] video[data-side="after"]');
  await expect(film).toBeVisible();
  await expect.poll(() => film.evaluate((v) => (v as HTMLVideoElement).readyState)).toBeGreaterThan(0);
  await expect(page.locator('[data-stage] img[data-side="after"]')).toHaveCount(0);
});

// The browser's own controls would sit under the layer notes are drawn on, so a film has the
// page's clock instead: a click on it plays or pauses, a drag notes the frame it paused on.
test("a film plays at a click, and a drag notes the frame it paused on", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('[data-picture="ee-page-look/checkout/desktop.webm"]').click();
  const film = page.locator('[data-stage] video[data-side="after"]');
  await expect.poll(() => film.evaluate((v) => (v as HTMLVideoElement).readyState)).toBeGreaterThan(0);
  await expect(film).not.toHaveAttribute("controls");
  const clock = page.locator("[data-stage] [data-clock]");
  const layer = page.locator('[data-stage] [data-draw="after"]');
  await expect(clock).toHaveText("0.00 s");

  await layer.click();
  await expect(clock).not.toHaveText("0.00 s");
  await layer.click();
  const paused = await clock.textContent();
  await page.waitForTimeout(300);
  await expect(clock).toHaveText(paused!);

  const box = (await layer.boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.2, box.y + box.height * 0.2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.5, { steps: 5 });
  await page.mouse.up();
  await expect(clock).toHaveText(paused!);
  await page.locator("textarea[data-words]").fill("The total jumps here");
  const made = page.waitForRequest((r) => r.method() === "POST" && r.url().endsWith("/notes"));
  await page.locator("[data-add-note]").click();
  expect((await made).postDataJSON().time).toBeCloseTo(Number.parseFloat(paused!), 2);
});

// Slower to see a change happen, faster to get through a long film; the speed holds from film
// to film, and < and > step it.
test("Space plays and pauses a film", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('[data-picture="ee-page-look/checkout/desktop.webm"]').click();
  const film = page.locator('[data-stage] video[data-side="after"]');
  await expect.poll(() => film.evaluate((v) => (v as HTMLVideoElement).readyState)).toBeGreaterThan(0);
  const clock = page.locator("[data-stage] [data-clock]");
  const scrolled = await page.evaluate(() => window.scrollY);
  await page.keyboard.press(" ");
  await expect(page.locator("[data-stage] [data-play]")).toHaveText("Pause");
  await expect(clock).not.toHaveText("0.00 s");
  await page.keyboard.press(" ");
  await expect(page.locator("[data-stage] [data-play]")).toHaveText("Play");
  expect(await page.evaluate(() => window.scrollY), "the page did not scroll").toBe(scrolled);
});

test("in full screen, a film stepped to plays at once", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('[data-picture="ee-page-look/checkout/desktop.webm"]').click();
  const play = page.locator("[data-stage] [data-play]");
  // On the page, a film stepped to waits for a click.
  await page.keyboard.press("j");
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/checkout/mobile.webm");
  await expect(play).toHaveText("Play");
  await page.keyboard.press("k");
  await page.keyboard.press("f");
  await expect(play).toHaveText("Play");
  await page.keyboard.press("j");
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/checkout/mobile.webm");
  await expect(play).toHaveText("Pause");
});

test("a film plays slower or faster at the speed picked", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('[data-picture="ee-page-look/checkout/desktop.webm"]').click();
  const film = page.locator('[data-stage] video[data-side="after"]');
  await expect.poll(() => film.evaluate((v) => (v as HTMLVideoElement).readyState)).toBeGreaterThan(0);
  const clock = page.locator("[data-stage] [data-clock]");
  const at = async () => Number.parseFloat((await clock.textContent())!);
  const layer = page.locator('[data-stage] [data-draw="after"]');
  // Plays for a moment at `speed`; how far the clock went.
  const played = async (speed: string) => {
    await page.locator(`[data-speed="${speed}"]`).click();
    await page.locator("[data-stage] .timeline").click({ position: { x: 1, y: 4 } });
    const from = await at();
    await layer.click();
    await page.waitForTimeout(400);
    await layer.click();
    return (await at()) - from;
  };
  const slow = await played("0.25");
  const fast = await played("4");
  expect(slow, "a quarter speed moves little").toBeLessThan(0.3);
  expect(fast, "four times moves far").toBeGreaterThan(slow * 4);

  await page.keyboard.press("<");
  await expect(page.locator('[data-speed="2"]')).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press(">");
  await page.keyboard.press(">");
  await expect(page.locator('[data-speed="4"]')).toHaveAttribute("aria-pressed", "true");
});

test("a note is edited here, or on every note linked with it", async ({ page }) => {
  const world = WORK["needs-change"]!.world;
  const live = world.live as import("../src/proof").Snapshot & { kind: "snapshot" };
  const first = live.notes![0]!;
  // The same note made again on the dark theme, linked with it.
  const linked = { ...live, notes: [first, { ...first, id: "n2", picture: "profile/desktop/dark.png", link: first.id }] };
  await show(page, WORK_PATH, { ...world, live: linked });
  const edits: unknown[] = [];
  await page.route("**/api/v2/notes/*", async (route) => {
    if (route.request().method() === "PATCH") edits.push({ url: route.request().url(), body: route.request().postDataJSON() });
    await route.fulfill({ json: { notes: [] } });
  });
  // One Save, on the linked note too unless a person unchecks it.
  await page.locator('[data-picture="ee-page-look/profile/desktop/light.png"]').click();
  await page.locator('[data-notes] [data-note="n1"] [data-edit]').click();
  await page.locator("textarea[data-edit-words]").fill("Keep the card where it was");
  await expect(page.locator(".edit-note .also")).toHaveText("Also on the linked note");
  await expect(page.locator("[data-save-linked]")).toBeChecked();
  await page.locator("[data-save-note]").click();
  await expect.poll(() => edits).toEqual([{ url: expect.stringContaining("/api/v2/notes/n1"), body: { words: "Keep the card where it was", linked: true } }]);
  await page.locator('[data-notes] [data-note="n1"] [data-edit]').click();
  await page.locator("textarea[data-edit-words]").fill("Only here");
  await page.locator("[data-save-linked]").uncheck();
  await page.locator("[data-save-note]").click();
  await expect.poll(() => edits.length).toBe(2);
  expect(edits[1]).toEqual({ url: expect.stringContaining("/api/v2/notes/n1"), body: { words: "Only here", linked: false } });
});

test("next and previous step on from a film", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('[data-picture="ee-page-look/checkout/desktop.webm"]').click();
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/checkout/desktop.webm");
  await page.getByRole("button", { name: "Next picture" }).click();
  await expect(page.locator(current)).not.toHaveAttribute("data-picture", "ee-page-look/checkout/desktop.webm");
  await page.getByRole("button", { name: "Previous picture" }).click();
  await expect(page.locator(current)).toHaveAttribute("data-picture", "ee-page-look/checkout/desktop.webm");
});
