import { test, expect } from "@playwright/test";
import { gotoApp, openWorkspace, selectFirstPassage } from "./helpers/workspace";
import { flushDiagnostics, watchPageDiagnostics } from "./helpers/diagnostics";

/**
 * Note draft lifecycle (reliability release): one inline editor, explicit
 * saves, retained drafts, truthful status, local recovery and guarded
 * departure — all against the browser preview's backend mirror.
 */

let consoleLines: ReturnType<typeof watchPageDiagnostics>;

test.beforeEach(async ({ page }) => {
  test.setTimeout(90_000);
  consoleLines = watchPageDiagnostics(page);
});

test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status !== testInfo.expectedStatus) {
    const forensics = await page.evaluate(() => {
      const w = window as unknown as Record<string, unknown>;
      const log = w.__FLEURON_IPC_LOG__ as undefined | { cmd: string; ok: boolean; err?: string }[];
      const tally: Record<string, string> = {};
      for (const entry of log ?? []) {
        tally[entry.cmd] = `${tally[entry.cmd] ?? ""}${entry.ok ? "." : "x"}`;
      }
      const useDrafts = w.useNoteDraftStore as undefined | { getState: () => Record<string, unknown> };
      const useProject = w.useProjectStore as undefined | { getState: () => Record<string, unknown> };
      return {
        tally,
        failures: (log ?? []).filter((e) => !e.ok).slice(-5),
        draftWorkspace: useDrafts?.getState().workspace ?? null,
        draftEntries: useDrafts ? Object.keys((useDrafts.getState().entries as object) ?? {}) : [],
        activeInline: useDrafts?.getState().activeInlineCodingId ?? null,
        loading: useProject?.getState().loading ?? null,
        project: Boolean(useProject?.getState().project),
      };
    }).catch(() => null);
    console.log("FORENSICS:", testInfo.title, JSON.stringify(forensics));
  }
  await flushDiagnostics(page, testInfo, consoleLines);
});

async function ipcCount(page: import("@playwright/test").Page, cmd: string) {
  return page.evaluate((command) => {
    const log = (window as unknown as { __FLEURON_IPC_LOG__?: { cmd: string; ok: boolean }[] })
      .__FLEURON_IPC_LOG__;
    return (log ?? []).filter((e) => e.cmd === command && e.ok).length;
  }, cmd);
}

function passage(page: import("@playwright/test").Page, n: number) {
  return page.getByRole("option", { name: new RegExp(`^Passage ${n}\\b`) });
}

async function openInlineNote(page: import("@playwright/test").Page) {
  const box = page.getByRole("textbox", { name: "Note content" });
  await expect(box).toBeVisible({ timeout: 15_000 });
  // Enabled means the draft entry landed (begin resolved): typing earlier
  // would race the resume. Wait for the backup to settle before acting so
  // clicks never land mid-settle.
  await expect(box).toBeEnabled({ timeout: 15_000 });
  return box;
}

async function waitForBackedUp(page: import("@playwright/test").Page) {
  await expect(page.getByText("Draft backed up locally")).toBeVisible({ timeout: 15_000 });
}

test.describe("one inline editor", () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page);
    await openWorkspace(page);
  });

  test("toggle, coding menu and bubble converge on a single editor", async ({ page }) => {
    // (a) Note toggle on a passage with a saved note.
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    const box = await openInlineNote(page);
    await expect(box).toHaveValue(/nothing is written down/);
    await expect(page.getByRole("textbox", { name: "Note content" })).toHaveCount(1);
    await passage(page, 3).getByRole("button", { name: "Collapse note" }).click();
    await expect(box).toBeHidden();

    // (b) Coding context menu on highlighted text.
    await passage(page, 3).locator("mark").first().click({ button: "right" });
    await page.getByRole("menuitem", { name: "Edit note" }).click();
    await openInlineNote(page);
    await expect(page.getByRole("textbox", { name: "Note content" })).toHaveCount(1);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("textbox", { name: "Note content" })).toBeHidden();

    // (c) Selection bubble on a fresh coding.
    await selectFirstPassage(page);
    await page.keyboard.press("c");
    const bubble = page.getByRole("dialog", { name: "Code this selection" });
    await expect(bubble).toBeVisible();
    await bubble.getByRole("textbox", { name: "Find or create a code" }).fill("Waiting list");
    await bubble.getByRole("button", { name: /^Waiting list/ }).first().click();
    await expect(bubble.getByRole("button", { name: "Add a note" })).toBeVisible();
    await bubble.getByRole("button", { name: "Add a note" }).click();
    await openInlineNote(page);
    await expect(page.getByRole("textbox", { name: "Note content" })).toHaveCount(1);
  });

  test("empty-note Add commits through the checked contract", async ({ page }) => {
    // A fresh coding of your own with no memo: the coding menu offers Add
    // note, which flows through the same inline editor and checked commit.
    await selectFirstPassage(page);
    await page.keyboard.press("c");
    const bubble = page.getByRole("dialog", { name: "Code this selection" });
    await expect(bubble).toBeVisible();
    await bubble.getByRole("textbox", { name: "Find or create a code" }).fill("Waiting list");
    await bubble.getByRole("button", { name: /^Waiting list/ }).first().click();
    await page.keyboard.press("Escape");
    await passage(page, 1).locator("mark").first().click({ button: "right" });
    await page.getByRole("menuitem", { name: "Add note" }).click();
    const box = await openInlineNote(page);
    await expect(box).toHaveValue("");
    await box.fill("Span-level reading, committed inline.");
    await page.getByRole("button", { name: "Save & close" }).click();
    await expect(page.getByRole("textbox", { name: "Note content" })).toBeHidden();
    await expect.poll(() => ipcCount(page, "save_note_draft"), { timeout: 15_000 }).toBeGreaterThan(0);
  });

  test("collapse retains the draft without prompting; reopen restores it", async ({ page }) => {
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    const box = await openInlineNote(page);
    await box.fill("distinct collapse draft 7f3a");
    await waitForBackedUp(page);
    await passage(page, 3).getByRole("button", { name: "Collapse note" }).click();
    await expect(box).toBeHidden();
    // No discard prompt appeared.
    await expect(page.getByRole("button", { name: "Keep editing" })).toHaveCount(0);
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    await expect(page.getByRole("textbox", { name: "Note content" })).toHaveValue(
      "distinct collapse draft 7f3a",
    );
  });

  test("switching interviews preserves the draft", async ({ page }) => {
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    const box = await openInlineNote(page);
    await box.fill("distinct interview-switch draft 9b1c");
    await page.getByRole("combobox", { name: "Current interview" }).selectOption("iv-2");
    await page.getByRole("combobox", { name: "Current interview" }).selectOption("iv-1");
    // Navigation never closes the editor and never reseeds it: the same
    // draft is still open with the unfinished words intact.
    const reopened = page.getByRole("textbox", { name: "Note content" });
    await expect(reopened).toBeVisible({ timeout: 15_000 });
    await expect(reopened).toHaveValue("distinct interview-switch draft 9b1c");
  });

  test("filtering hides the editor and returning restores it", async ({ page }) => {
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    const box = await openInlineNote(page);
    await box.fill("distinct filter draft 4d2e");
    // Filter to a code that only matches another passage: the editor hides
    // with its passage, the draft stays.
    await page.getByRole("button", { name: "Filter passages" }).click();
    await page.getByRole("menuitem", { name: /Learning by transgression/ }).click();
    await expect(page.getByRole("textbox", { name: "Note content" })).toBeHidden();
    await page.getByRole("button", { name: "Filter passages" }).click();
    await page.getByRole("menuitem", { name: "All passages" }).click();
    const restored = page.getByRole("textbox", { name: "Note content" });
    await expect(restored).toBeVisible({ timeout: 15_000 });
    await expect(restored).toHaveValue("distinct filter draft 4d2e");
  });

  test("Mod+Enter saves and closes from the keyboard", async ({ page }) => {
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    const box = await openInlineNote(page);
    await box.fill("keyboard-saved draft");
    await box.press(process.platform === "darwin" ? "Meta+Enter" : "Control+Enter");
    await expect(page.getByRole("textbox", { name: "Note content" })).toBeHidden();
    await expect.poll(() => ipcCount(page, "save_note_draft"), { timeout: 15_000 }).toBeGreaterThan(0);
  });
});

test.describe("truthful status and recovery", () => {
  test("a rejected commit keeps the draft open with Retry, announced once", async ({ page }) => {
    await gotoApp(page, "/?fixture=note-commit-fail");
    await openWorkspace(page);
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    const box = await openInlineNote(page);
    await box.fill("words that must not be lost");
    await page.getByRole("button", { name: "Save & close" }).click();
    await expect(page.getByText(/Could not save note/)).toBeVisible();
    await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
    // Still open, text intact, no false Saved.
    await expect(box).toBeVisible();
    await expect(box).toHaveValue("words that must not be lost");
    // The failure is sticky and inline — one announcement, no toast loop,
    // and no toast host mounted for a notice that was never raised.
    await expect(page.getByRole("region", { name: "Notifications" })).toHaveCount(0);
    await expect(page.getByRole("alert")).toHaveCount(1);
  });

  test("a changed saved note opens the comparison, never overwrites", async ({ page }) => {
    await gotoApp(page, "/?fixture=note-changed");
    await openWorkspace(page);
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    const box = await openInlineNote(page);
    await box.fill("my version of the reading");
    await page.getByRole("button", { name: "Save & close" }).click();
    await page.getByRole("button", { name: "Compare" }).click();
    const dialog = page.getByRole("dialog", { name: "This note changed elsewhere" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole("textbox", { name: "Your unfinished draft" })).toHaveValue(
      "my version of the reading",
    );
    await expect(dialog.getByRole("textbox", { name: "Current saved note" })).not.toHaveValue(
      "my version of the reading",
    );
    await dialog.getByRole("button", { name: "Save my version" }).click();
    await expect(dialog).toBeHidden({ timeout: 15_000 });
  });

  test("a missing target is reported and never recreated", async ({ page }) => {
    await gotoApp(page, "/?fixture=note-missing");
    await openWorkspace(page);
    // The coding menu still offers Edit note; beginning reports it missing.
    await passage(page, 3).locator("mark").first().click({ button: "right" });
    await page.getByRole("menuitem", { name: "Edit note" }).click();
    await expect(page.getByText(/no longer available/)).toBeVisible({ timeout: 15_000 });
  });

  test("interview autosave never reports Saved for a rejected write", async ({ page }) => {
    await gotoApp(page, "/?fixture=note-commit-fail");
    await openWorkspace(page);
    await page.getByRole("button", { name: "More actions" }).click();
    await page.getByRole("menuitem", { name: "Notes on this interview" }).click();
    const sheet = page.getByRole("dialog", { name: "Notes on this interview" });
    await expect(sheet).toBeVisible();
    await sheet.getByRole("textbox", { name: "Interview analytic memo" }).fill("autosave must be truthful");
    await expect(sheet.getByText("Not saved")).toBeVisible({ timeout: 15_000 });
    await expect(sheet.getByText("Saved", { exact: true })).toHaveCount(0);
    // Inline and sticky — no toast loop for an unattended failure.
    await expect(page.getByRole("region", { name: "Notifications" })).toHaveCount(0);
  });
});

test.describe("unfinished notes and departure", () => {
  test.beforeEach(async ({ page }) => {
    await gotoApp(page);
    await openWorkspace(page);
  });

  test("recovery panel lists, expands, copies and discards a draft", async ({ page }) => {
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    const box = await openInlineNote(page);
    await box.fill("recoverable draft c81d");
    await expect(page.getByText("Draft backed up locally")).toBeVisible({ timeout: 15_000 });

    await page.getByRole("button", { name: "More actions" }).click();
    await page.getByRole("menuitem", { name: /Unfinished notes/ }).click();
    const panel = page.getByRole("dialog", { name: "Unfinished notes" });
    await expect(panel).toBeVisible();
    await panel.getByRole("button", { name: /Passage 3/ }).click();
    await expect(panel.getByText("recoverable draft c81d")).toBeVisible();
    await expect(panel.getByRole("button", { name: "Copy text" })).toBeVisible();
    // Copy raises the one toast through the single Notifications host.
    await panel.getByRole("button", { name: "Copy text" }).click();
    const notices = page.getByRole("region", { name: "Notifications" });
    await expect(notices).toHaveCount(1);
    await expect(notices.getByRole("status")).toHaveCount(1);
    await panel.getByRole("button", { name: "Discard" }).click();
    await page.getByRole("button", { name: "Discard", exact: true }).last().click();
    await expect(panel.getByText("No unfinished notes")).toBeVisible({ timeout: 15_000 });
  });

  test("closing with a draft offers save, discard and cancel", async ({ page }) => {
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    await (await openInlineNote(page)).fill("departure draft e22a");

    // Cancel keeps everything open.
    await page.getByRole("button", { name: "More actions" }).click();
    await page.getByRole("menuitem", { name: "Close study" }).click();
    const dialog = page.getByRole("dialog", { name: "Unfinished notes" });
    await expect(dialog).toBeVisible({ timeout: 15_000 });
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByRole("listbox", { name: "Transcript passages" })).toBeVisible();

    // Discard closes without saving.
    await page.getByRole("button", { name: "More actions" }).click();
    await page.getByRole("menuitem", { name: "Close study" }).click();
    await expect(dialog).toBeVisible({ timeout: 15_000 });
    await dialog.getByRole("button", { name: "Discard changes" }).click();
    await expect(page.getByRole("button", { name: /Start a local study/ })).toBeVisible({
      timeout: 15_000,
    });
  });

  test("save-all closes with the committed text", async ({ page }) => {
    await passage(page, 3).getByRole("button", { name: "Expand note" }).click();
    await (await openInlineNote(page)).fill("departure save-all draft");

    await page.getByRole("button", { name: "More actions" }).click();
    await page.getByRole("menuitem", { name: "Close study" }).click();
    const dialog = page.getByRole("dialog", { name: "Unfinished notes" });
    await expect(dialog).toBeVisible({ timeout: 15_000 });
    await dialog.getByRole("button", { name: "Save all" }).click();
    await expect(page.getByRole("button", { name: /Start a local study/ })).toBeVisible({
      timeout: 15_000,
    });
    await expect.poll(() => ipcCount(page, "save_note_draft"), { timeout: 15_000 }).toBeGreaterThan(0);
  });
});
