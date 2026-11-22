// Comparing a picture with any earlier run, in the page: the strip of runs above the picture,
// the pair each dot shows, and the comparison the page runs itself, which must answer as the
// CLI does for the same files.

import { type Page, expect, test } from "@playwright/test";
import { show } from "./host";
import { LONG_STRIP, STRIP, WORK, WORK_PATH, answerOf } from "./states";

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
  await expect(page.locator("[data-pair-says]")).toHaveText("Changed since your look in 1 place.");
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
        return compared(b, a, f);
      },
      [before, after, film] as const,
    );
  expect(await inPage("/files/r2/looks/profile/after.png", "/files/r3/looks/profile/after.png", false)).toEqual(answerOf("still.json"));
  expect(await inPage("/files/r2/looks/checkout/after.webm", "/files/r3/looks/checkout/after.webm", true)).toEqual(answerOf("film.json"));
});
