import { test, expect } from "@playwright/test";
import { gotoApp, openWorkspace, selectFirstPassage } from "./helpers/workspace";

/**
 * The editing side rail is gone: every Add/Edit route converges on the one
 * inline passage-note editor docked below its source passage.
 */
test.describe("Workspace Notes inline editor", () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page);
    await openWorkspace(page);
  });

  test("no editing rail exists; closing the inline editor keeps it closed", async ({ page }) => {
    await expect(page.locator('[data-testid="memo-panel"]')).toHaveCount(0);

    const passage = page.locator("article").nth(2);
    await passage.click();

    const noteBtn = page.getByRole("button", { name: /Edit note|Add a note/i });
    if (await noteBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
      await noteBtn.click();
      const box = page.getByRole("textbox", { name: "Note content" });
      await expect(box).toBeVisible();

      // Collapse via Escape: retained, not discarded, rail never appears.
      await page.keyboard.press("Escape");
      await expect(box).toBeHidden();
      await expect(page.locator('[data-testid="memo-panel"]')).toHaveCount(0);

      // Navigating passages must NOT reopen the editor automatically.
      await selectFirstPassage(page);
      await page.waitForTimeout(200);
      await expect(box).toBeHidden();
      await expect(page.locator('[data-testid="memo-panel"]')).toHaveCount(0);
    }
  });

  test("exactly one Notifications host, mounted only while notices exist", async ({ page }) => {
    // App.tsx owns the single ToastStack; the workspace mounts none. With no
    // notices raised there is no region at all — one failure raises exactly
    // one region with one announcement (see note-draft-lifecycle.spec.ts).
    await expect(page.getByRole("region", { name: "Notifications" })).toHaveCount(0);
  });
});

test.describe("Workspace stress fixture", () => {
  test("loads without layout overflow", async ({ page }) => {
    test.setTimeout(60_000);
    await gotoApp(page, "/?fixture=stress");
    await openWorkspace(page);

    const transcriptScroller = page.locator('[data-testid="transcript-scroller"]');
    await expect(transcriptScroller).toBeVisible();

    const articles = transcriptScroller.locator("article");
    await expect.poll(() => articles.count(), { timeout: 30_000 }).toBe(320);

    // Verify document does not scroll even with 320 passages
    const docScroll = await page.evaluate(() => {
      const el = document.scrollingElement || document.documentElement;
      return { scrollTop: el.scrollTop, scrollHeight: el.scrollHeight, clientHeight: el.clientHeight };
    });
    expect(docScroll.scrollTop).toBe(0);
    expect(docScroll.scrollHeight).toBe(docScroll.clientHeight);
  });
});
