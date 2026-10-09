import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const SCREENSHOTS = resolve(
  import.meta.dirname,
  "../../../evidence/final/screens",
);
const DARK_SCREENSHOTS = resolve(
  import.meta.dirname,
  "../../../evidence/theme/screens",
);

async function openCase(
  page: Page,
  scenario: string,
  route: string,
  heading: string,
) {
  const title = page.getByRole("heading", { name: heading, level: 1 });
  // The dev server can drop module requests under load
  // (ERR_INSUFFICIENT_RESOURCES); one reload recovers without hiding failures.
  for (let attempt = 0; attempt < 4; attempt += 1) {
    if (attempt > 0) await page.waitForTimeout(750);
    await page.goto(`/?jev_case=${scenario}#${route}`);
    const shown = await title
      .waitFor({ state: "visible", timeout: 5_000 })
      .then(() => true)
      .catch(() => false);
    if (shown) return;
  }
  await expect(title).toBeVisible();
}

async function expectNoWindowOverflow(page: Page) {
  const geometry = await page.evaluate(() => ({
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    documentWidth: document.documentElement.scrollWidth,
    documentHeight: document.documentElement.scrollHeight,
    bodyWidth: document.body.scrollWidth,
    bodyHeight: document.body.scrollHeight,
  }));
  expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.innerWidth);
  expect(geometry.bodyWidth).toBeLessThanOrEqual(geometry.innerWidth);
  expect(geometry.documentHeight).toBeLessThanOrEqual(geometry.innerHeight);
  expect(geometry.bodyHeight).toBeLessThanOrEqual(geometry.innerHeight);
}

async function fixtureAudit(page: Page) {
  return page.evaluate(() => {
    const audit = (
      globalThis as typeof globalThis & {
        __CUTOKYO_FIXTURE_AUDIT__?: () => {
          proxyActive: boolean;
          resumeRequests: string[];
          sessionIds: string[];
        };
      }
    ).__CUTOKYO_FIXTURE_AUDIT__;
    if (audit === undefined) throw new Error("Fixture audit is unavailable.");
    return audit();
  });
}

test("onboarding remains local-only and opens useful empty history", async ({
  page,
}) => {
  await openCase(
    page,
    "onboarding-empty-history",
    "/onboarding",
    "Set up Cutokyo",
  );
  await expect(page.getByText(/Proxy/)).toHaveCount(0);
  await page.getByRole("checkbox", { name: /stored unencrypted/ }).check();
  await page.getByRole("button", { name: "Skip for now" }).click();
  await expect(
    page.getByRole("heading", { name: "Overview", level: 1 }),
  ).toBeVisible();
  await page
    .getByRole("navigation", { name: "Primary navigation" })
    .getByRole("link", { name: "Sessions", exact: true })
    .click();
  await expect(page.getByText("No sessions yet")).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Review capture setup" }),
  ).toBeVisible();
});

test("search detail and exact resume preserve native identity", async ({
  page,
}) => {
  await openCase(page, "search-detail-resume", "/sessions", "Sessions");
  await page
    .getByRole("searchbox", { name: "Search session content" })
    .fill("JEV exact resume needle 73A9");
  await expect(page.getByText("1 session", { exact: true })).toBeVisible();
  await page
    .getByRole("link", { name: "Reconcile usage capture", exact: true })
    .click();
  await expect(page.getByRole("heading", { name: "Timeline" })).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();
  const dialog = page.getByRole("dialog", { name: "Resume in Claude Code?" });
  await expect(dialog.getByText(/^Folder:/)).toBeVisible();
  await dialog.getByRole("button", { name: "Resume", exact: true }).click();
  await expect
    .poll(async () => (await fixtureAudit(page)).resumeRequests)
    .toEqual(["claude-jev-73A9"]);
});

test("retention and delete-all cancellation are previewed; one-session deletion lives in session detail", async ({
  page,
}) => {
  await openCase(
    page,
    "retention-delete-confirmation",
    "/data",
    "Data controls",
  );
  await expect(page.getByLabel("Keep sessions for")).toHaveValue("forever");
  await page.getByLabel("Keep sessions for").selectOption("30");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByText(
      /No history was deleted; review a cleanup preview separately/,
    ),
  ).toBeVisible();
  expect((await fixtureAudit(page)).sessionIds).toEqual([
    "delete-me-73A9",
    "keep-me-73A9",
  ]);
  await page.getByRole("button", { name: "Preview 30-day cleanup" }).click();
  const retention = page.getByRole("dialog", {
    name: "Preview 30-day retention policy",
  });
  await expect(retention.getByText("delete-me-73A9")).toBeVisible();
  await retention.getByRole("button", { name: "Cancel" }).click();

  await expect(
    page.getByRole("button", { name: "Delete only this" }),
  ).toHaveCount(0);
  await expect(page.getByText("Delete one session")).toHaveCount(0);
  await page.goto(
    "/?jev_case=retention-delete-confirmation#/sessions/delete-me-73A9",
  );
  await page.getByRole("button", { name: "Delete session" }).click();
  const one = page.getByRole("dialog", { name: "Delete only this session?" });
  await expect(one.getByText(/not physical secure erasure/i)).toBeVisible();
  await one.getByRole("button", { name: "Delete selected session" }).click();
  await expect
    .poll(async () => (await fixtureAudit(page)).sessionIds)
    .toEqual(["keep-me-73A9"]);
  await page.goto("/?jev_case=retention-delete-confirmation#/data");
  await expect(
    page.getByRole("heading", { name: "Data controls", level: 1 }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Delete all…" }).click();
  const all = page.getByRole("dialog", { name: "Delete all local history?" });
  await all.getByRole("button", { name: "Cancel — keep all history" }).click();
  expect((await fixtureAudit(page)).sessionIds).toEqual(["keep-me-73A9"]);
});

test("settings shows no proxy or guard controls while the build has no proxy", async ({
  page,
}) => {
  await openCase(page, "proxy-capture-consent", "/settings", "Settings");
  await expect(page.getByRole("switch", { name: /guard/i })).toHaveCount(0);
  await expect(page.locator('a[href="#/guards"]')).toHaveCount(0);
  await expect(page.getByText(/proxy/i)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Enable proxy" })).toHaveCount(
    0,
  );
  expect((await fixtureAudit(page)).proxyActive).toBe(false);
});

test("writer-lock retry does not clear unrelated quarantine", async ({
  page,
}) => {
  await openCase(page, "degraded-health-recovery", "/health", "Health");
  await expect(page.getByText("2 issues")).toBeVisible();
  const writer = page.locator("article").filter({ hasText: "Writer lock" });
  const quarantine = page.locator("article").filter({ hasText: "Quarantine" });
  await expect(writer).toBeVisible();
  await expect(quarantine).toBeVisible();
  await writer.getByRole("button", { name: "Retry writer lock" }).click();
  await expect(page.getByText("1 issue", { exact: true })).toBeVisible();
  await expect(writer).toHaveCount(0);
  await expect(
    page.getByRole("list", { name: "Other checks" }).getByText("Writer lock"),
  ).toBeVisible();
  await expect(quarantine).toBeVisible();
  await page.getByText("Technical details").click();
  await expect(page.getByText("spool:malformed-73A9")).toBeVisible();
});

test("MCP source edits persist without changing plugin or search installations", async ({
  page,
}) => {
  await openCase(page, "mcp-plugin-inventory", "/inventory", "Agent tools");
  const docs = page.locator("article").filter({ hasText: "docs-73A9" });
  await docs.getByRole("button", { name: "Manage docs-73A9" }).click();
  const manager = page.getByRole("dialog", { name: "Manage docs-73A9" });
  await manager.getByRole("tab", { name: "File" }).click();
  const source = manager.getByRole("textbox", { name: "Source content" });
  await expect(source).not.toHaveValue("");
  const updated = JSON.stringify(
    { command: "local-docs", args: ["--offline"], env: {} },
    null,
    2,
  );
  await source.fill(updated);
  await manager.getByRole("button", { name: "Save changes" }).click();
  await expect(manager).not.toBeVisible();
  await docs.getByRole("button", { name: "Manage docs-73A9" }).click();
  await manager.getByRole("tab", { name: "File" }).click();
  await expect(source).toHaveValue(updated);
  await manager.getByRole("button", { name: "Done", exact: true }).click();
  await expect(page.getByText("Cutokyo search MCP")).toBeVisible();
  const plugin = page
    .locator("article")
    .filter({ hasText: "fixture-processor-73A9" });
  await plugin.getByRole("button", { name: "Verification" }).click();
  const dialog = page.getByRole("dialog", { name: /Plugin verification/ });
  await expect(dialog.getByText("normalized_records:read")).toBeVisible();
  await expect(dialog.getByText("30 seconds")).toBeVisible();
  await expect(
    dialog.getByText(
      /does not claim portable filesystem or network sandboxing/i,
    ),
  ).toBeVisible();
});

test("keyboard navigation exposes focus and restores route-heading focus", async ({
  page,
}) => {
  await openCase(page, "visual-keyboard-consistency", "/dashboard", "Overview");
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to content" });
  await expect(skip).toBeFocused();
  await expect(skip).toHaveCSS("transform", "matrix(1, 0, 0, 1, 0, 0)");
  await page.keyboard.press("Tab");
  await expect(page.getByRole("link", { name: "Overview" })).toBeFocused();
  await page.keyboard.press("Tab");
  const sessionsNavigation = page
    .getByRole("navigation", { name: "Primary navigation" })
    .getByRole("link", { name: "Sessions", exact: true });
  await expect(sessionsNavigation).toBeFocused();
  await page.keyboard.press("Enter");
  const heading = page.getByRole("heading", { name: "Sessions", level: 1 });
  await expect(heading).toBeVisible();
  await expect(heading).toBeFocused();
});

test("dashboard and setup confirmation pass browser accessibility checks", async ({
  page,
}) => {
  await openCase(page, "populated-dashboard", "/dashboard", "Overview");
  const dashboard = await new AxeBuilder({ page }).analyze();
  expect(dashboard.violations).toEqual([]);

  await openCase(
    page,
    "onboarding-empty-history",
    "/onboarding",
    "Set up Cutokyo",
  );
  await page
    .getByRole("button", { name: "More actions for Claude Code" })
    .click();
  await page.getByRole("menuitem", { name: "Install Claude Code" }).click();
  await expect(
    page.getByRole("dialog", { name: "Install Claude Code?" }),
  ).toBeVisible();
  const consent = await new AxeBuilder({ page }).analyze();
  expect(consent.violations).toEqual([]);
});

test("System follows the OS; explicit appearance survives route navigation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await page.emulateMedia({ colorScheme: "dark" });
  await openCase(page, "visual-keyboard-consistency", "/settings", "Settings");
  await expectNoWindowOverflow(page);
  const root = page.locator("html");
  const color = page.locator('meta[name="theme-color"]');
  await expect(
    page.getByRole("button", { name: "System", pressed: true }),
  ).toBeVisible();
  await expect(root).toHaveAttribute("data-theme", "dark");
  await expect(color).toHaveAttribute("content", "#111411");

  await page.emulateMedia({ colorScheme: "light" });
  await expect(root).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: "Dark" }).click();
  await expect(
    page.getByRole("button", { name: "Dark", pressed: true }),
  ).toBeVisible();
  await expect(root).toHaveAttribute("data-theme", "dark");
  await expect(
    page.locator("#main-content").getByText("Appearance set to dark."),
  ).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.emulateMedia({ colorScheme: "light" });
  await expect(root).toHaveAttribute("data-theme", "dark");

  await page.getByRole("link", { name: "Overview" }).click();
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  await page.getByRole("link", { name: "Settings" }).click();
  await expect(
    page.getByRole("button", { name: "Dark", pressed: true }),
  ).toBeVisible();
  const light = page.getByRole("button", { name: "Light" });
  await light.focus();
  await expect(light).toBeFocused();
  await page.keyboard.press("Space");
  await expect(
    page.getByRole("button", { name: "Light", pressed: true }),
  ).toBeVisible();
  await expect(
    page.locator("#main-content").getByText("Appearance set to light."),
  ).toBeVisible();
  await expect(light).toBeFocused();
  await expect(root).toHaveAttribute("data-theme", "light");
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(root).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: "System" }).click();
  await expect(root).toHaveAttribute("data-theme", "dark");
  await page.emulateMedia({ colorScheme: "light" });
  await expect(root).toHaveAttribute("data-theme", "light");
  // The theme changes optimistically. Audit only after the save receipt and
  // enabled controls prove the preference operation has actually completed.
  await expect(
    page.locator("#main-content").getByText("Appearance set to system."),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "System" })).toHaveAttribute(
    "aria-disabled",
    "false",
  );
  const a11y = await new AxeBuilder({ page }).analyze();
  expect(a11y.violations).toEqual([]);
});

test("saved dark appears before app render; failed save restores System", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const watcher = new MutationObserver(() => {
      if (document.querySelector("#root > *") !== null) {
        (window as Window & { __firstAppTheme?: string }).__firstAppTheme =
          document.documentElement.dataset.theme ?? "unset";
        watcher.disconnect();
      }
    });
    watcher.observe(document, { childList: true, subtree: true });
  });
  await openCase(page, "appearance-dark", "/settings", "Settings");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  expect(
    await page.evaluate(
      () => (window as Window & { __firstAppTheme?: string }).__firstAppTheme,
    ),
  ).toBe("dark");
  await expect(
    page.getByRole("button", { name: "Dark", pressed: true }),
  ).toBeVisible();

  await openCase(page, "appearance-save-error", "/settings", "Settings");
  await page.getByRole("button", { name: "Dark" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Could not save appearance",
  );
  await expect(
    page.getByRole("button", { name: "System", pressed: true }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("a concurrent appearance write reloads the saved winner instead of old Dark", async ({
  page,
}) => {
  await mkdir(DARK_SCREENSHOTS, { recursive: true });
  await page.setViewportSize({ width: 900, height: 700 });
  await openCase(page, "appearance-conflict", "/settings", "Settings");
  await expect(
    page.getByRole("button", { name: "Dark", pressed: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Light" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Saved appearance is system; your light choice was not saved.",
  );
  await expect(
    page.getByRole("button", { name: "System", pressed: true }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expectNoWindowOverflow(page);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: resolve(DARK_SCREENSHOTS, "settings-conflict-900x700.png"),
  });
});

test("slow saved Dark prevents a wrong-theme app render past 500ms", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const watcher = new MutationObserver(() => {
      if (document.querySelector("#root .app-shell") !== null) {
        (window as Window & { __firstAppTheme?: string }).__firstAppTheme =
          document.documentElement.dataset.theme ?? "unset";
        watcher.disconnect();
      }
    });
    watcher.observe(document, { childList: true, subtree: true });
  });
  await page.goto("/?jev_case=appearance-dark-delayed#/settings");
  await expect(
    page
      .getByRole("status")
      .filter({ hasText: "Waiting for your saved appearance" }),
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "Settings" })).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: /Continue with device appearance/ }),
  ).toBeVisible();
  await page.evaluate(() => {
    const release = (
      window as Window & { __CUTOKYO_FIXTURE_RELEASE_APPEARANCE__?: () => void }
    ).__CUTOKYO_FIXTURE_RELEASE_APPEARANCE__;
    if (release === undefined)
      throw new Error("Appearance fixture gate is missing.");
    release();
  });
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  expect(
    await page.evaluate(
      () => (window as Window & { __firstAppTheme?: string }).__firstAppTheme,
    ),
  ).toBe("dark");
});

test("forced colors retains distinct visible chart textures and raw values", async ({
  page,
}) => {
  await mkdir(DARK_SCREENSHOTS, { recursive: true });
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.emulateMedia({ forcedColors: "active" });
  await openCase(page, "populated-dashboard", "/dashboard", "Overview");
  await expect(page.getByRole("table", { name: /Raw values/ })).toBeVisible();
  const marks = await page
    .locator(".usage-chart__segment")
    .evaluateAll((elements) =>
      elements.map((element) => ({
        name: element.getAttribute("aria-label"),
        texture: getComputedStyle(element).backgroundImage,
        forcedColorAdjust: getComputedStyle(element).forcedColorAdjust,
      })),
    );
  expect(marks.map((mark) => mark.name)).toEqual([
    expect.stringMatching(/^Claude Code: [\d,]+ tokens$/),
    expect.stringMatching(/^Codex: [\d,]+ tokens$/),
    expect.stringMatching(/^OpenCode: [\d,]+ tokens$/),
  ]);
  expect(marks.map((mark) => mark.forcedColorAdjust)).toEqual([
    "none",
    "none",
    "none",
  ]);
  expect(marks[0]?.texture).toContain("45deg");
  expect(marks[1]?.texture).toContain("135deg");
  expect(marks[2]?.texture).toBe("none");
  await page.screenshot({
    path: resolve(DARK_SCREENSHOTS, "dashboard-forced-colors-1280x800.png"),
  });
});

test("dark dashboard, empty history, settings, health and resume fit all desktop sizes", async ({
  context,
}) => {
  await mkdir(DARK_SCREENSHOTS, { recursive: true });

  const sizes = [
    { width: 900, height: 700 },
    { width: 1280, height: 800 },
    { width: 1536, height: 960 },
  ] as const;

  for (const size of sizes) {
    const page = await context.newPage();
    await page.emulateMedia({ colorScheme: "dark" });
    await page.setViewportSize(size);
    const suffix = `${size.width}x${size.height}`;

    const root = page.locator("html");
    await openCase(page, "populated-dashboard", "/dashboard", "Overview");
    await expect(root).toHaveAttribute("data-theme", "dark");
    await expect(page.getByRole("table", { name: /Raw values/ })).toBeVisible();
    await expectNoWindowOverflow(page);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({
      path: resolve(DARK_SCREENSHOTS, `dashboard-dark-${suffix}.png`),
    });

    await openCase(page, "empty-history", "/sessions", "Sessions");
    await expect(page.getByText("No sessions yet")).toBeVisible();
    await expect(root).toHaveAttribute("data-theme", "dark");
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(DARK_SCREENSHOTS, `empty-history-dark-${suffix}.png`),
    });

    await openCase(page, "appearance-dark", "/settings", "Settings");
    await expect(
      page.getByRole("button", { name: "Dark", pressed: true }),
    ).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(DARK_SCREENSHOTS, `settings-dark-${suffix}.png`),
    });

    await openCase(page, "degraded-health-recovery", "/health", "Health");
    await expect(page.getByText("2 issues")).toBeVisible();
    await expect(root).toHaveAttribute("data-theme", "dark");
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(DARK_SCREENSHOTS, `health-degraded-dark-${suffix}.png`),
    });

    await openCase(
      page,
      "search-detail-resume",
      "/sessions/session-claude-73A9",
      "Reconcile usage capture",
    );
    await page.getByRole("button", { name: "Resume" }).click();
    const resume = page.getByRole("dialog", {
      name: "Resume in Claude Code?",
    });
    await expect(resume.getByText(/^Folder:/)).toBeVisible();
    await expect(
      resume.getByRole("button", { name: "Resume", exact: true }),
    ).toBeEnabled();
    await expect(root).toHaveAttribute("data-theme", "dark");
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(DARK_SCREENSHOTS, `resume-dialog-dark-${suffix}.png`),
    });
    await page.close();
  }
});

test("captures the seven representative states at all required desktop sizes", async ({
  context,
}) => {
  test.setTimeout(90_000);
  await mkdir(SCREENSHOTS, { recursive: true });
  const sizes = [
    { width: 900, height: 700 },
    { width: 1280, height: 800 },
    { width: 1536, height: 960 },
  ] as const;

  for (const size of sizes) {
    // A fresh page per size keeps the dev server from exhausting resources.
    const page = await context.newPage();
    await page.setViewportSize(size);
    const suffix = `${size.width}x${size.height}`;

    await openCase(
      page,
      "onboarding-empty-history",
      "/onboarding",
      "Set up Cutokyo",
    );
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `onboarding-${suffix}.png`),
    });

    await openCase(page, "empty-history", "/sessions", "Sessions");
    await expect(page.getByText("No sessions yet")).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `empty-history-${suffix}.png`),
    });

    await openCase(page, "search-detail-resume", "/sessions", "Sessions");
    await expect(
      page.getByRole("link", { name: "Reconcile usage capture", exact: true }),
    ).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `sessions-list-${suffix}.png`),
    });

    await openCase(page, "populated-dashboard", "/dashboard", "Overview");
    await expect(page.getByRole("table", { name: /Raw values/ })).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `dashboard-${suffix}.png`),
    });

    await openCase(
      page,
      "search-detail-resume",
      "/sessions/session-claude-73A9",
      "Reconcile usage capture",
    );
    await page.getByRole("button", { name: "Resume" }).click();
    const resume = page.getByRole("dialog", {
      name: "Resume in Claude Code?",
    });
    await expect(resume.getByText(/^Folder:/)).toBeVisible();
    await expect(
      resume.getByRole("button", { name: "Resume", exact: true }),
    ).toBeVisible();
    await expect(
      resume.getByRole("button", { name: "Resume", exact: true }),
    ).toBeEnabled();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `session-resume-${suffix}.png`),
    });

    await openCase(page, "mcp-plugin-inventory", "/inventory", "Agent tools");
    await expect(page.getByText("docs-73A9")).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `inventory-mcp-${suffix}.png`),
    });

    await page.getByRole("button", { name: /^Warnings/ }).click();
    await expect(
      page.getByRole("region", { name: "Warnings for Agent tools" }),
    ).toBeVisible();
    await page.screenshot({
      path: resolve(SCREENSHOTS, `warnings-open-${suffix}.png`),
    });
    await page.keyboard.press("Escape");

    await page.getByRole("button", { name: "Manage docs-73A9" }).click();
    await expect(
      page.getByRole("dialog", { name: "Manage docs-73A9" }),
    ).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `inventory-manage-${suffix}.png`),
    });
    await page
      .getByRole("dialog", { name: "Manage docs-73A9" })
      .getByRole("button", { name: /Install to…/ })
      .click();
    await page.screenshot({
      path: resolve(SCREENSHOTS, `inventory-install-menu-${suffix}.png`),
    });
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");

    await openCase(
      page,
      "retention-delete-confirmation",
      "/data",
      "Data controls",
    );
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `data-controls-${suffix}.png`),
    });

    await openCase(page, "proxy-capture-consent", "/settings", "Settings");
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `settings-${suffix}.png`),
    });

    await openCase(
      page,
      "onboarding-empty-history",
      "/onboarding",
      "Set up Cutokyo",
    );
    await page.getByRole("checkbox", { name: /stored unencrypted/ }).check();
    await page.getByRole("checkbox", { name: "Capture Claude Code" }).check();
    const install = page.getByRole("button", { name: "Install selected" });
    await expect(install).toBeEnabled();
    await expect(install).toHaveCSS("opacity", "1");
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `setup-${suffix}.png`),
    });
    await page.getByRole("button", { name: "Install selected" }).click();
    await expect(
      page.getByRole("dialog", { name: "Install Claude Code?" }),
    ).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `setup-confirm-${suffix}.png`),
    });

    await openCase(page, "degraded-health-recovery", "/health", "Health");
    await expect(page.getByText("2 issues")).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `health-degraded-${suffix}.png`),
    });
    await page.close();
  }
});

test("warnings live in one shell badge; no workspace top bar remains", async ({
  page,
}) => {
  await openCase(page, "mcp-plugin-inventory", "/inventory", "Agent tools");
  await expect(page.locator(".topbar")).toHaveCount(0);
  await expect(page.getByText("All local projects")).toHaveCount(0);
  await expect(page.getByText("AI egress")).toHaveCount(0);
  await expect(page.getByRole("link", { name: /analysis/i })).toHaveCount(0);
  const trigger = page.getByRole("button", { name: /^Warnings/ });
  await expect(trigger).toContainText("1");
  await expect(page.locator(".route-notice")).toHaveCount(0);
  await trigger.click();
  const panel = page.getByRole("region", { name: "Warnings for Agent tools" });
  await expect(panel).toContainText(
    "One project plugin has incomplete runtime",
  );
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await openCase(page, "empty-history", "/sessions", "Sessions");
  await expect(
    page.getByRole("button", { name: /^Warnings/ }),
  ).not.toContainText(/\d/);
});

test("agent tool rows show description, kind symbols, harness marks and an aligned toolbar", async ({
  page,
}) => {
  await openCase(page, "mcp-plugin-inventory", "/inventory", "Agent tools");
  await expect(page.locator("summary", { hasText: "Source" })).toHaveCount(0);
  await expect(page.getByText("Supporting asset of skill bundle")).toHaveCount(
    0,
  );
  await expect(page.getByText(/Shared native source/)).toHaveCount(0);
  const kinds = await page
    .locator(".inventory-row__icon")
    .evaluateAll(
      (nodes) =>
        new Set(nodes.map((node) => `${node.getAttribute("aria-label")}`)).size,
    );
  expect(kinds).toBeGreaterThanOrEqual(4);
  await expect(
    page
      .locator(".inventory-card")
      .first()
      .locator(".harness-chips svg.harness-mark"),
  ).not.toHaveCount(0);
  const boxes = await page.locator(".toolbar > *").evaluateAll((nodes) =>
    nodes.map((node) => {
      const box = node.getBoundingClientRect();
      return { top: box.top, bottom: box.bottom };
    }),
  );
  expect(boxes.length).toBeGreaterThanOrEqual(3);
  // Controls on one visual line share the same top and bottom; wrapped lines
  // each hold a uniform 38px control height.
  for (const box of boxes) {
    expect(Math.abs(box.bottom - box.top - 38)).toBeLessThanOrEqual(1);
    const sameLine = boxes.filter(
      (other) => Math.abs(other.top - box.top) < 20,
    );
    for (const other of sameLine) {
      expect(Math.abs(other.top - box.top)).toBeLessThanOrEqual(1);
      expect(Math.abs(other.bottom - box.bottom)).toBeLessThanOrEqual(1);
    }
  }
  await page.getByRole("button", { name: "Manage docs-73A9" }).click();
  const dialog = page.getByRole("dialog", { name: "Manage docs-73A9" });
  await dialog.getByRole("button", { name: /Install to…/ }).click();
  await expect(
    dialog.getByRole("button", { name: "Install to Codex" }),
  ).toBeVisible();
  await dialog.getByRole("tab", { name: "Details" }).click();
  await expect(dialog.locator("summary", { hasText: "Source" })).toBeVisible();
});
