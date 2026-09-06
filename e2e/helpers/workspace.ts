import { expect, type Locator, type Page } from "@playwright/test";

/** Wait for Vite + React to paint the shell (first compile can be slow). */
export async function gotoApp(page: Page, path = "/") {
  page.on("pageerror", (err) => console.error("BROWSER_ERROR:", err));
  page.on("console", (msg) => {
    if (msg.type() === "error") console.error("BROWSER_CONSOLE_ERR:", msg.text());
  });
  await page.goto(path, { waitUntil: "domcontentloaded" });
  await expect(page.locator("#root").locator("> *").first()).toBeAttached({ timeout: 45_000 });
}

export async function openWorkspace(page: Page) {
  const passages = page.getByRole("listbox", { name: "Transcript passages" });
  const clickAndObserveWorkspace = async (locator: Locator) => {
    // Opening a study replaces the WelcomeScreen synchronously after the mock
    // IPC reply. Playwright can therefore observe the clicked element detach
    // while it is completing the action; the passage assertion below is the
    // authoritative outcome, not that transient element's post-click state.
    await locator.click({ timeout: 5_000 }).catch(() => {});
  };
  try {
    await passages.waitFor({ state: "visible", timeout: 4000 });
    await ensureCoderReady(page);
    return;
  } catch {
    // not open yet, continue to open via welcome screen
  }

  // App initialization and the asynchronous home-library refresh can overlap
  // on a cold browser preview. Retry the visible user route after it settles;
  // this keeps the test on the same public opening flow rather than reaching
  // into the store or mock internals. An open already in flight (the opening
  // overlay) is waited out, never clicked on top of: a second open would
  // replace the connection and tear down the workspace the first one built.
  const opening = () => page.getByText("Opening project…");
  for (let attempt = 0; attempt < 3; attempt += 1) {
    if (await opening().isVisible({ timeout: 1500 }).catch(() => false)) {
      if (await passages.isVisible({ timeout: 30_000 }).catch(() => false)) {
        await ensureCoderReady(page);
        return;
      }
      continue;
    }
    const workLocally = page.getByRole("button", { name: /Work locally/i });
    if (await workLocally.isVisible({ timeout: 1500 }).catch(() => false)) {
      await clickAndObserveWorkspace(workLocally);
    }

    const recent = page.getByRole("button", {
      name: /Sample Study/,
    }).first();
    if (await recent.isVisible({ timeout: 5000 }).catch(() => false)) {
      await clickAndObserveWorkspace(recent);
    } else if (await opening().isVisible({ timeout: 1000 }).catch(() => false)) {
      if (await passages.isVisible({ timeout: 30_000 }).catch(() => false)) {
        await ensureCoderReady(page);
        return;
      }
      continue;
    } else {
      const openBtn = page.getByRole("button", {
        name: /Open a folder|Open an existing study/i,
      }).first();
      if (await openBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
        await clickAndObserveWorkspace(openBtn);
      } else {
        const genericOpen = page.getByRole("button", { name: /Open/i }).first();
        if (await genericOpen.isVisible().catch(() => false)) {
          await clickAndObserveWorkspace(genericOpen);
        }
      }
    }

    if (await passages.isVisible({ timeout: 20_000 }).catch(() => false)) {
      await ensureCoderReady(page);
      return;
    }
  }

  await expect(passages).toBeVisible({ timeout: 10_000 });
  await ensureCoderReady(page);
}

export async function ensureCoderReady(page: Page) {
  const identity = page.getByRole("dialog", { name: "Who's coding?" });
  if (await identity.isVisible({ timeout: 1000 }).catch(() => false)) {
    await identity.getByRole("button", { name: "Ada Lovelace" }).click();
    return;
  }

  const select = page.getByRole("combobox", { name: "Active coder" });
  if (await select.isVisible({ timeout: 1000 }).catch(() => false)) {
    const val = await select.inputValue().catch(() => "");
    if (!val) {
      await select.selectOption("Ada Lovelace").catch(() => {});
    }
  }
}

export async function selectFirstPassage(page: Page) {
  await page.getByRole("option", { name: /^Passage 1\b/ }).click();
}

export async function openCodingBubble(page: Page) {
  await ensureCoderReady(page);
  await selectFirstPassage(page);
  await page.keyboard.press("c");
  await expect(
    page.getByRole("dialog", { name: "Code this selection" }),
  ).toBeVisible();
}

export async function dismissTopDialog(page: Page) {
  await page.keyboard.press("Escape");
  const close = page.getByRole("button", { name: "Close", exact: true });
  if (await close.isVisible().catch(() => false)) {
    await close.click({ timeout: 2000 }).catch(() => {});
  }
}
