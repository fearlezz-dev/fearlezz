// Comparing a picture with any earlier run, in the page: the strip of runs above the picture,
// the pair each dot shows, and the comparison the page runs itself, which must answer as the
// CLI does for the same files.

import { type Page, expect, test } from "@playwright/test";
import { serve, settled, show } from "./host";
import { ALIGNED, LONG_STRIP, STRIP, WORK, WORK_PATH, answerOf } from "./states";

const dots = (page: Page) => page.locator("[data-strip] [data-dot]");
const dot = (page: Page, key: string) => page.locator(`[data-strip] [data-dot="${key}"] button`);
const stage = (page: Page) => page.locator("[data-stage]");
const caption = (page: Page, side: "before" | "after") => page.locator(`[data-stage] figcaption[data-caption="${side}"]`);
/** What comparing the pair came to, once the page has compared it. */
const compared = (page: Page) => expect(page.locator("[data-pair-says]")).not.toHaveAttribute("data-pair-says", "comparing");

test("the strip lists the base, each earlier run and now, runs that showed the same picture as one", async ({ page }) => {
  await show(page, WORK_PATH, STRIP.world);
  await expect(dots(page)).toHaveCount(4);
  expect(await dots(page).evaluateAll((all) => all.map((d) => d.getAttribute("data-dot")))).toEqual(["base", "r1", "l2", "now"]);
  // The local run and the CI run of the second push showed the same, so they are one dot.
  await expect(dot(page, "l2")).toContainText("Your look ×2");
  await expect(dot(page, "r1")).toContainText("Run");
  await expect(dot(page, "base")).toContainText("Base");
  await expect(dot(page, "now")).toContainText("Now");
});

test("a picture to look again opens compared with your look, in the Look again colour", async ({ page }) => {
  await show(page, WORK_PATH, STRIP.world);
  await expect(stage(page)).toHaveAttribute("data-key", STRIP.still);
  await expect(stage(page)).toHaveAttribute("data-against", "look");
  await expect(dot(page, "l2")).toHaveAttribute("aria-pressed", "true");
  await expect(caption(page, "before")).toHaveText("Your look");
  await expect(caption(page, "after")).toHaveText("Now");
  await compared(page);
  // Who said it looks right, said once: above the picture, not again below it.
  await expect(page.locator("[data-pair-says]")).toHaveText(/^Changed since mara's look at [0-9a-f]{7} in 1 place\.$/);
  await expect(page.locator(".verdict [data-again]")).toHaveCount(0);
  const again = await page.evaluate(() => {
    const probe = document.createElement("i");
    probe.style.color = "var(--again)";
    document.body.append(probe);
    const colour = getComputedStyle(probe).color;
    probe.remove();
    return colour;
  });
  await expect(caption(page, "before")).toHaveCSS("color", again);
  await expect(page.locator("[data-stage] .mark").first()).toHaveCSS("outline-color", again);
  await expect(page.locator('[data-strip] [data-dot="now"]')).toHaveAttribute("data-since-look", "");
});

test("[ and ] step the strip to the older and newer run, and B goes back to the base", async ({ page }) => {
  await show(page, WORK_PATH, STRIP.world);
  const picked = () => page.locator('[data-strip] [data-dot] button[aria-pressed="true"]:not(:disabled)');
  await expect(picked()).toHaveCount(1);
  await expect(page.locator('[data-strip] [data-dot="l2"] button')).toHaveAttribute("aria-pressed", "true");
  for (const [key, to] of [["[", "r1"], ["[", "base"], ["[", "base"], ["]", "r1"], ["]", "l2"], ["]", "l2"], ["b", "base"]] as const) {
    await page.keyboard.press(key);
    await expect(dot(page, to), `${key} reaches ${to}`).toHaveAttribute("aria-pressed", "true");
  }
  await expect(stage(page)).not.toHaveAttribute("data-against");
  // With a keyboard, each way to compare says its key on its button, and the strip says its
  // own; a touch screen shows no keys.
  const keys = test.info().project.name !== "mobile";
  await expect(page.locator('[data-mode="slider"] kbd')).toBeVisible({ visible: keys });
  await expect(page.locator('[data-mode="slider"] kbd')).toHaveText("2");
  await expect(page.locator("[data-strip-keys]")).toBeVisible({ visible: keys });
});

test("a dot shows its run's picture beside now, and the Look again line goes back to your look", async ({ page }) => {
  await show(page, WORK_PATH, STRIP.world);
  await dot(page, "base").click();
  await expect(stage(page)).not.toHaveAttribute("data-against");
  await expect(caption(page, "before")).toHaveText("Before");
  await expect(caption(page, "after")).toHaveText("After");
  // CI's one outline, on each side.
  await expect(page.locator("[data-stage] .mark")).toHaveCount(2);

  await dot(page, "r1").click();
  await expect(stage(page)).toHaveAttribute("data-against", "run");
  await expect(caption(page, "before")).toHaveText("Run");
  await expect(page.locator('[data-stage] img[data-side="before"]')).toHaveAttribute("src", /\/files\/r1\//);
  await compared(page);
  await expect(page.locator("[data-pair-says]")).toHaveAttribute("data-pair-says", "changed");

  await page.locator("[data-compare-look]").click();
  await expect(stage(page)).toHaveAttribute("data-against", "look");
  await expect(page.locator("[data-compare-look]")).toHaveCount(0);
});

test("a film's timeline draws what its check said happened: its steps, and a load whose timing may change", async ({ page }) => {
  await show(page, WORK_PATH, STRIP.world);
  await page.locator(`[data-picture="${STRIP.film}"]`).click();
  await compared(page);
  for (const side of ["before", "after"]) {
    const lane = page.locator(`[data-stage] [data-lane="${side}"]`);
    expect(await lane.locator("[data-mark]").evaluateAll((all) => all.map((m) => m.getAttribute("data-mark")))).toEqual(["opened", "paid"]);
    await expect(lane.locator('[data-span="loading"][data-timing="free"]')).toHaveCount(1);
  }
  await expect(page.locator("[data-stage] [data-step]")).toHaveText("loading");
  // Both films are two seconds long, so 45% along is 0.9 s, after paying.
  const timeline = page.locator("[data-stage] .timeline");
  const box = (await timeline.boundingBox())!;
  await timeline.click({ position: { x: box.width * 0.45, y: 4 } });
  await expect(page.locator("[data-stage] [data-step]")).toHaveText("paid");
});

test("two films play on one timeline aligned at their marks, each holding dimmed where it has no footage", async ({ page }) => {
  await show(page, WORK_PATH, ALIGNED);
  await page.locator(`[data-picture="${STRIP.film}"]`).click();
  await dot(page, "base").click();
  await expect(stage(page)).not.toHaveAttribute("data-against");
  const lane = (side: string) => page.locator(`[data-stage] [data-lane="${side}"]`);
  // The step each film reached is drawn at one place on the timeline.
  const paid = async (side: string) => (await lane(side).locator('[data-mark="paid"]').boundingBox())!.x;
  expect(Math.abs((await paid("before")) - (await paid("after")))).toBeLessThan(1.5);
  // The before film has no footage before it, the after film none at the end.
  await expect(lane("before").locator("[data-gap]")).toHaveCount(1);
  await expect(lane("after").locator("[data-gap]")).toHaveCount(1);

  const timeline = page.locator("[data-stage] .timeline");
  const box = (await timeline.boundingBox())!;
  const film = (side: string) => page.locator(`[data-stage] video[data-side="${side}"]`).first();
  const at = (side: string) => film(side).evaluate((v: HTMLVideoElement) => v.currentTime);
  // 10% along, 0.26 s: the after film plays, the before film holds its first frame, dimmed.
  await timeline.click({ position: { x: box.width * 0.1, y: 4 } });
  await expect(film("before")).toHaveAttribute("data-held", "");
  await expect(page.locator('[data-stage] [data-no-footage="before"]').first()).toBeVisible();
  await expect(film("after")).not.toHaveAttribute("data-held");
  await expect(page.locator("[data-stage] .mark")).toHaveCount(0);
  await expect.poll(() => at("before")).toBeLessThan(0.05);
  await expect.poll(() => at("after")).toBeCloseTo(0.26, 1);
  // 60% along, 1.56 s: both play, each at its own time past the mark.
  await timeline.click({ position: { x: box.width * 0.6, y: 4 } });
  await expect(film("before")).not.toHaveAttribute("data-held");
  await expect.poll(() => at("before")).toBeCloseTo(0.96, 1);
  await expect.poll(() => at("after")).toBeCloseTo(1.56, 1);
});

test("in full screen, two films play once both can play through and the page has compared them", async ({ page }) => {
  await show(page, WORK_PATH, STRIP.world);
  await page.locator(`[data-picture="${STRIP.film}"]`).click();
  // Notes what stood the moment the films began to play.
  const watch = () =>
    page.evaluate(() => {
      const w = window as unknown as { started?: { ready: number[]; says: string | null } };
      delete w.started;
      const play = () => document.querySelector("[data-stage] [data-play]");
      const seen = new MutationObserver(() => {
        if (play()?.textContent !== "Pause" || w.started) return;
        const films = [...document.querySelectorAll<HTMLVideoElement>("[data-stage] video")];
        w.started = { ready: films.map((v) => v.readyState), says: document.querySelector("[data-pair-says]")?.getAttribute("data-pair-says") ?? null };
      });
      seen.observe(document.body, { subtree: true, childList: true, characterData: true });
    });
  const started = () => page.waitForFunction(() => (window as unknown as { started?: unknown }).started).then((h) => h.jsonValue() as Promise<{ ready: number[]; says: string | null }>);

  // A film stepped to in full screen plays at once.
  await page.keyboard.press("k");
  await page.keyboard.press("f");
  await watch();
  await page.keyboard.press("j");
  await expect(stage(page)).toHaveAttribute("data-key", STRIP.film);
  let at = await started();
  expect(at.ready, "both films could play through").toEqual([4, 4]);
  await expect(page.locator("[data-busy-says]")).toHaveCount(0);
  expect(at.says, "compared with your look first").not.toBe("comparing");

  // Another run: the page compares it before the films play.
  await page.keyboard.press(" ");
  await watch();
  await dot(page, "r1").click();
  at = await started();
  expect(at.ready).toEqual([4, 4]);
  expect(at.says).toBe("changed");
});

test("the film to look at next is compared with your look ahead, so it opens compared", async ({ page }) => {
  await show(page, WORK_PATH, STRIP.world);
  await expect(stage(page)).toHaveAttribute("data-key", STRIP.still);
  await compared(page);
  // Time for the film and its comparison to be made ahead.
  await page.waitForTimeout(3000);
  await page.locator(`[data-picture="${STRIP.film}"]`).click();
  const at = await page.evaluate(() => document.querySelector("[data-pair-says]")?.getAttribute("data-pair-says"));
  expect(at, "compared before it was shown").not.toBe("comparing");
});

test("a film pair's lanes grow in once, and are not drawn again as its films load or its run changes", async ({ page }) => {
  // How the lanes grow in is motion, which stills turn off (playwright.config.ts).
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const socket = await serve(page, STRIP.world);
  // Films that take a moment, as they do over a real connection.
  await page.route(/\.webm/, async (route) => {
    await new Promise((done) => setTimeout(done, 1500));
    await route.fallback();
  });
  await page.goto(WORK_PATH);
  await settled(page, true);
  // Every lane that grows in, kept to see whether it is still the one shown.
  await page.evaluate(() => {
    const w = window as unknown as { grown: Element[] };
    w.grown = [];
    document.addEventListener("animationstart", (e) => {
      if (e.animationName === "grow") w.grown.push(e.target as Element);
    });
  });
  await page.locator(`[data-picture="${STRIP.film}"]`).click();
  await dot(page, "r1").click();
  await compared(page);
  await expect.poll(() => page.locator("[data-stage] video").evaluateAll((all) => all.map((v) => (v as HTMLVideoElement).readyState))).toEqual([4, 4]);
  // Another check of the run ends, as checks do while CI runs.
  const live = STRIP.world.live as { seq: number };
  socket.send({ kind: "event", seq: live.seq + 1, run: "r3", change: { kind: "finished", check: { id: "clippy", outcome: "passed", seconds: 1 } } });
  await page.waitForTimeout(800);
  const grown = await page.evaluate(() => (window as unknown as { grown: Element[] }).grown.map((e) => e.isConnected));
  expect(grown.length, "the lanes grew in").toBeGreaterThan(0);
  expect(grown.every(Boolean), "each lane that grew in is still the one shown").toBe(true);
});

// The strip as shown, a fold as + and the runs it holds.
const stops = (page: Page) =>
  page.locator("[data-strip] li").evaluateAll((all) => all.map((li) => li.getAttribute("data-dot") ?? `+${li.getAttribute("data-fold")}:${li.textContent}`));

test("a long history keeps the base, your look, the run before now and now, and folds the rest", async ({ page }, info) => {
  await show(page, WORK_PATH, LONG_STRIP);
  await expect(dot(page, "l2")).toHaveAttribute("aria-pressed", "true");
  // A phone has no room for the run before now, so it folds too.
  const after = info.project.name === "mobile" ? ["+y1:+3"] : ["+y1:+2", "y3"];
  expect(await stops(page)).toEqual(["base", "+r1:+5", "l2", ...after, "now"]);
  const fold = page.locator('[data-strip] [data-fold="r1"] button');
  await expect(fold).toHaveAccessibleName("Show 5 more runs");
  await fold.click();
  expect(await stops(page)).toEqual(["base", "r1", "x1", "x2", "x3", "x4", "l2", ...after, "now"]);
  // A run picked from an opened fold stays, though the next picture starts folded again.
  await dot(page, "x2").click();
  await expect(caption(page, "before")).toHaveText("Run");
  await expect(page.locator('[data-stage] img[data-side="before"]')).toHaveAttribute("src", /\/files\/x2\//);
});

test("a picture with only the base and now shows no strip", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  await expect(stage(page)).toBeVisible();
  await expect(page.locator("[data-strip]")).toHaveCount(0);
});

// The page runs the CLI's own comparison as wasm on what the browser decodes: for the same
// still and film it must answer exactly what `fearlezz setup compare --json` does, outlines,
// states and timeline alike.
test("the page compares a still and a film as the CLI does", async ({ page }) => {
  await show(page, WORK_PATH, STRIP.world);
  const inPage = (before: string, after: string, film: boolean) =>
    page.evaluate(
      async ([b, a, f]) => {
        // The dev server serves the page's own module, worker and wasm.
        const module = "/src/compare.ts";
        const { compared } = (await import(/* @vite-ignore */ module)) as typeof import("../src/compare");
        return compared({ url: b }, { url: a }, f);
      },
      [before, after, film] as const,
    );
  expect(await inPage("/files/r2/looks/profile/after.png", "/files/r3/looks/profile/after.png", false)).toEqual(answerOf("still.json"));
  expect(await inPage("/files/r2/looks/checkout/after.webm", "/files/r3/looks/checkout/after.webm", true)).toEqual(answerOf("film.json"));
});
