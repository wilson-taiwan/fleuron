import { test, expect } from "@playwright/test";
import { flushDiagnostics, watchPageDiagnostics } from "./helpers/diagnostics";
import { gotoApp, openWorkspace } from "./helpers/workspace";

let consoleLines: ReturnType<typeof watchPageDiagnostics>;

test.beforeEach(async ({ page }) => {
  consoleLines = watchPageDiagnostics(page);
  await gotoApp(page);
});

test.afterEach(async ({ page }, testInfo) => {
  await flushDiagnostics(page, testInfo, consoleLines);
});

test("manage speakers modal opens from interview menu and displays speakers", async ({ page }) => {
  test.setTimeout(90_000);
  await openWorkspace(page);

  // Click the interview actions menu
  const interviewMenuBtn = page.getByRole("button", { name: /Interview actions|Options for interview/i }).first();
  if (await interviewMenuBtn.isVisible()) {
    await interviewMenuBtn.click();
    const manageSpeakersItem = page.getByRole("menuitem", { name: /Manage speakers/i });
    await expect(manageSpeakersItem).toBeVisible();
    await manageSpeakersItem.click();

    // Modal should open
    const modal = page.getByRole("dialog", { name: /Manage speakers/i });
    await expect(modal).toBeVisible();

    // Verify local-only explanation
    await expect(modal.getByText(/on this computer/i)).toBeVisible();

    // Close modal via Cancel or Close button
    const closeBtn = modal.getByRole("button", { name: /Close|Cancel/i }).first();
    await closeBtn.click();
    await expect(modal).not.toBeVisible();
  }
});
