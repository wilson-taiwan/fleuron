import { test, expect } from "@playwright/test";
import { flushDiagnostics, watchPageDiagnostics } from "./helpers/diagnostics";
import { gotoApp } from "./helpers/workspace";

let consoleLines: ReturnType<typeof watchPageDiagnostics>;

test.beforeEach(async ({ page }) => {
  consoleLines = watchPageDiagnostics(page);
  await gotoApp(page);
});

test.afterEach(async ({ page }, testInfo) => {
  await flushDiagnostics(page, testInfo, consoleLines);
});

test("welcome screen displays Fleuron branding and backup restore trigger", async ({ page }) => {
  // Canonical Fleuron mark is rendered on welcome screen
  const mark = page.locator("svg").filter({ has: page.locator("path") }).first();
  await expect(mark).toBeVisible();

  // Restore backup button/card is present on home
  const restoreTrigger = page.getByRole("button", { name: /Restore backup|Restore a study backup/i });
  await expect(restoreTrigger).toBeVisible();
});

test("removal modal presents capability-based choices for studies", async ({ page }) => {
  // If a recent study is visible, trigger its menu or removal
  const studyMenuButton = page.getByRole("button", { name: /Options for|Study actions/i }).first();
  if (await studyMenuButton.isVisible()) {
    await studyMenuButton.click();
    const removeOption = page.getByRole("menuitem", { name: /Remove|Leave|Delete/i });
    if (await removeOption.isVisible()) {
      await removeOption.click();

      const dialog = page.getByRole("dialog");
      await expect(dialog).toBeVisible();

      // Check that capability-based actions are rendered
      const radioOrButton = dialog.getByRole("radio").or(dialog.getByRole("button"));
      await expect(radioOrButton.first()).toBeVisible();

      // Can safely cancel
      const cancelBtn = dialog.getByRole("button", { name: /Cancel|Keep/i });
      await cancelBtn.click();
      await expect(dialog).not.toBeVisible();
    }
  }
});
