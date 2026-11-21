// Every page of the proof host in every state worth seeing, light and dark, as stills that
// Fearlezz compares with the base commit's. A still is named `<screen>/<theme>`; the project
// (desktop or mobile) is its viewport.

import { type Page, expect, test } from "@playwright/test";
import { settled, show } from "./host";
import { REPOSITORY_PATH, REPOSITORY_STATES, WORK, WORK_PATH } from "./states";

const THEMES = ["light", "dark"] as const;

/** Takes a still of the whole page in each theme, as `<screen>/<theme>`. */
/** Stills of `screen` in each theme: the whole page, or only the window (`fullPage` false) for
 * what covers the window, such as full screen. */
async function stills(page: Page, screen: string, fullPage = true) {
  for (const theme of THEMES) {
    await page.emulateMedia({ colorScheme: theme });
    await expect(page).toHaveScreenshot([...screen.split("/"), `${theme}.png`], { fullPage });
  }
}

for (const [name, { path, world }] of Object.entries(WORK)) {
  test(`work page: ${name}`, async ({ page }) => {
    await show(page, path ?? WORK_PATH, world);
    await stills(page, `work/${name}`);
  });
}

for (const [name, world] of Object.entries(REPOSITORY_STATES)) {
  test(`repository page: ${name}`, async ({ page }) => {
    await show(page, REPOSITORY_PATH, world);
    await stills(page, `repository/${name}`);
  });
}

// A swatch sits on its viewport's line, a film's too (whose play mark once took the margin of
// the film clock's class).
test("every swatch sits centred on its viewport's line", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  const off = await page.evaluate(() =>
    [...document.querySelectorAll(".tile .variants .viewport")].flatMap((v) => {
      const mid = (e: Element) => {
        const r = e.getBoundingClientRect();
        return r.y + r.height / 2;
      };
      const line = v.querySelector(".vp");
      return line ? [...v.querySelectorAll(".variant")].map((s) => Math.abs(mid(s) - mid(line))) : [];
    }),
  );
  expect(off.length).toBeGreaterThan(0);
  expect(Math.max(...off)).toBeLessThan(1);
});

// Every kind of swatch side by side: stills and films, light and dark, desktop and mobile, with
// and without looks right (said here on the desktop viewport).
test("work page: every kind of swatch", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('.variant[data-picture="ee-page-look/profile/desktop/light.webm"]').click();
  await page.keyboard.press("Alt+KeyY");
  await page.locator('.variant[data-picture="ee-page-look/profile/mobile/dark.png"]').click();
  await settled(page, true);
  const tile = page.locator('[data-screen="ee-page-look/profile"]');
  for (const theme of THEMES) {
    await page.emulateMedia({ colorScheme: theme });
    await expect(tile).toHaveScreenshot(["work", "swatches", `${theme}.png`]);
  }
});

test("work page: a held Y in full screen", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('.variant[data-picture="ee-page-look/profile/desktop/dark.png"]').click();
  await page.keyboard.press("f");
  await page.keyboard.down("y");
  await expect(page.locator("[data-say-menu]")).toBeVisible();
  await page.keyboard.up("y");
  await page.waitForTimeout(300);
  await stills(page, "work/looks-right-held", false);
});

test("work page: how far looks right reaches", async ({ page }) => {
  await show(page, WORK_PATH, WORK["no-before"]!.world);
  await page.locator('.variant[data-picture="ee-page-look/profile/desktop/dark.png"]').click();
  await page.locator("[data-say-more]").click();
  await stills(page, "work/looks-right-menu", false);
});

test("work page: a weakening's change opened", async ({ page }) => {
  await show(page, WORK_PATH, WORK["weaker-proof"]!.world);
  await page.locator("[data-show-change]").first().click();
  await stills(page, "work/weaker-proof-open");
});

// Each way to compare a changed picture, picked with its key as a person would.
for (const [key, mode] of [["2", "slider"], ["3", "onion"], ["4", "diff"]] as const) {
  test(`work page: a changed picture compared by ${mode}`, async ({ page }) => {
    await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
    await page.keyboard.press(key);
    await settled(page, true);
    await stills(page, `work/compare-${mode}`);
  });
}

test("work page: the text picked", async ({ page }) => {
  await show(page, WORK_PATH, WORK["waits-for-you"]!.world);
  await page.locator('[data-picture="ee-page-look/manifests/deployment.yaml"]').click();
  await settled(page, true);
  await stills(page, "work/text-picked");
});

// A note made on one variant offered on another of the same screen.
test("work page: a note of another variant", async ({ page }) => {
  await show(page, WORK_PATH, WORK["needs-change"]!.world);
  await page.locator('[data-picture="ee-page-look/profile/desktop/dark.png"]').click();
  await stills(page, "work/note-elsewhere");
});

test("work page: full screen", async ({ page }) => {
  await show(page, WORK_PATH, WORK["needs-change"]!.world);
  await page.keyboard.press("f");
  await stills(page, "work/full-screen", false);
});

test("work page: full screen, zoomed in", async ({ page, isMobile }) => {
  await show(page, WORK_PATH, WORK["needs-change"]!.world);
  await page.keyboard.press("f");
  if (isMobile) for (const _ of [1, 2, 3]) await page.keyboard.press("+");
  else {
    // Pinched in on the banner the note is about.
    const after = (await page.locator('[data-stage] [data-side="after"]').boundingBox())!;
    await page.mouse.move(after.x + after.width * 0.3, after.y + after.height * 0.16);
    await page.keyboard.down("Control");
    for (const _ of [1, 2, 3]) await page.mouse.wheel(0, -100);
    await page.keyboard.up("Control");
  }
  // Settled where the zoom eases to.
  await page.waitForTimeout(300);
  await stills(page, "work/zoomed", false);
});
