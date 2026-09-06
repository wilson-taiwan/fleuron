import { test, expect } from "@playwright/test";
import { flushDiagnostics, watchPageDiagnostics } from "./helpers/diagnostics";
import {
  dismissTopDialog,
  gotoApp,
  openCodingBubble,
  openWorkspace,
} from "./helpers/workspace";

let consoleLines: ReturnType<typeof watchPageDiagnostics>;

test.beforeEach(async ({ page }) => {
  consoleLines = watchPageDiagnostics(page);
  await gotoApp(page);
  await page.evaluate(() => {
    const w = window as unknown as { __FLEURON_CLEAR_IPC_LOG__?: () => void };
    w.__FLEURON_CLEAR_IPC_LOG__?.();
  });
});

test.afterEach(async ({ page }, testInfo) => {
  await flushDiagnostics(page, testInfo, consoleLines);
});

async function ipcCount(page: import("@playwright/test").Page, cmd: string) {
  return page.evaluate((command) => {
    const log = (window as unknown as { __FLEURON_IPC_LOG__?: { cmd: string; ok: boolean }[] })
      .__FLEURON_IPC_LOG__;
    return (log ?? []).filter((e) => e.cmd === command && e.ok).length;
  }, cmd);
}

async function applyExistingCode(page: import("@playwright/test").Page, name: string) {
  const bubble = page.getByRole("dialog", { name: "Code this selection" });
  await bubble.getByRole("textbox", { name: "Find or create a code" }).fill(name);
  await bubble.getByRole("button", { name, exact: true }).click({ force: true });
  await dismissTopDialog(page);
  await expect
    .poll(() => ipcCount(page, "mutate_coding_edge"), { timeout: 15_000 })
    .toBeGreaterThan(0);
  await expect
    .poll(
      () =>
        page.evaluate(
          () =>
            (
              window as unknown as {
                useProjectStore?: { getState: () => { undoStack: unknown[] } };
              }
            ).useProjectStore?.getState().undoStack.length ?? 0,
        ),
      { timeout: 15_000 },
    )
    .toBeGreaterThan(0);
}

test("open workspace and code from the bubble", async ({ page }) => {
  test.setTimeout(90_000);
  await openWorkspace(page);
  await openCodingBubble(page);
  await applyExistingCode(page, "Waiting list");
});

test("undo removes a coding action", async ({ page }) => {
  test.setTimeout(90_000);
  await openWorkspace(page);
  const passage2 = page.locator("article").nth(1);
  await passage2.click();
  await page.keyboard.press("c");
  await applyExistingCode(page, "Waiting list");

  await page.evaluate(async () => {
    const w = window as unknown as {
      useProjectStore?: { getState: () => { undoLastCoding: () => Promise<void> } };
    };
    if (w.useProjectStore) {
      await w.useProjectStore.getState().undoLastCoding();
    }
  });
  await expect(
    page.getByRole("option", { name: /^Passage 2\b.*Not yet coded/ }),
  ).toBeVisible({ timeout: 10_000 });
});

test("codebook filters passages and passage notes open inline", async ({ page }) => {
  test.setTimeout(90_000);
  await openWorkspace(page);

  const filter = page.getByRole("button", {
    name: "Filter passages",
  });
  const passages = page.getByRole("listbox", { name: "Transcript passages" });
  await filter.click();
  await page.getByRole("menuitem", { name: /Unwritten rules/ }).click();
  await expect(passages.getByRole("option")).toHaveCount(1);

  await page.getByRole("button", { name: "Filter passages" }).click();
  await page.getByRole("menuitem", { name: "All passages" }).click();
  await expect(passages.getByRole("option")).toHaveCount(12);

  const note = page.getByRole("button", { name: "Expand note" }).first();
  await note.click();
  const inlineBox = passages.getByRole("textbox", { name: "Note content" });
  await expect(inlineBox).toBeVisible();
  await inlineBox.fill("Updated from the inline note.");
  await passages.getByRole("button", { name: "Save & close" }).click();
  // Inline passage notes commit through the checked write contract, not the
  // legacy memo command.
  await expect
    .poll(() => ipcCount(page, "save_note_draft"), { timeout: 15_000 })
    .toBeGreaterThan(0);
});
