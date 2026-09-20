import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const SCREENSHOTS = resolve(
  import.meta.dirname,
  "../../../evidence/final/screens",
);

async function openCase(
  page: Page,
  scenario: string,
  route: string,
  heading: string,
) {
  await page.goto(`/?jev_case=${scenario}#${route}`);
  await expect(
    page.getByRole("heading", { name: heading, level: 1 }),
  ).toBeVisible();
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
          outboundAnalysisRequests: number;
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
    "See your agent work without sending it away",
  );
  await expect(
    page.getByText("Proxy capture and AI analysis stay disabled."),
  ).toBeVisible();
  await page.getByRole("checkbox", { name: /I understand/ }).check();
  await page.getByRole("button", { name: /Complete local-only setup/ }).click();
  await expect(
    page.getByRole("heading", { name: "Overview", level: 1 }),
  ).toBeVisible();
  await expect(page.getByText("Local only")).toBeVisible();
  await expect(page.getByText("Proxy off")).toBeVisible();
  await expect(page.getByText("AI egress on confirmation only")).toBeVisible();
  await page
    .getByRole("navigation", { name: "Primary navigation" })
    .getByRole("link", { name: "Sessions", exact: true })
    .click();
  await expect(page.getByText("History is empty—not zero")).toBeVisible();
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
  const dialog = page.getByRole("dialog", {
    name: "Resume this exact native session?",
  });
  await expect(dialog.getByRole("code")).toHaveText("claude-jev-73A9");
  await dialog.getByRole("button", { name: "Resume in Claude Code" }).click();
  await expect
    .poll(async () => (await fixtureAudit(page)).resumeRequests)
    .toEqual(["claude-jev-73A9"]);
});

test("retention, one-session deletion, and delete-all cancellation are previewed", async ({
  page,
}) => {
  await openCase(
    page,
    "retention-delete-confirmation",
    "/data",
    "Data controls",
  );
  await page.getByRole("button", { name: "Preview 30-day policy" }).click();
  const retention = page.getByRole("dialog", {
    name: "Preview 30-day retention policy",
  });
  await expect(retention.getByText("delete-me-73A9")).toBeVisible();
  await retention.getByRole("button", { name: "Cancel" }).click();

  const deleteRow = page
    .locator("article")
    .filter({ hasText: "delete-me-73A9" });
  await deleteRow.getByRole("button", { name: "Delete only this" }).click();
  const one = page.getByRole("dialog", { name: "Delete exactly one session?" });
  await expect(one.getByText(/not physical secure erasure/i)).toBeVisible();
  await one
    .getByRole("button", { name: "Confirm one-session deletion" })
    .click();
  await expect(page.getByText("keep-me-73A9", { exact: true })).toBeVisible();
  await expect(page.getByText("delete-me-73A9", { exact: true })).toHaveCount(
    0,
  );

  await page.getByRole("button", { name: "Delete all…" }).click();
  const all = page.getByRole("dialog", { name: "Delete all local history?" });
  await all.getByRole("button", { name: "Cancel — keep all history" }).click();
  expect((await fixtureAudit(page)).sessionIds).toEqual(["keep-me-73A9"]);
});

test("proxy preview distinguishes unavailable coverage and does not activate on cancel", async ({
  page,
}) => {
  await openCase(
    page,
    "guard-proxy-coverage-language",
    "/guards",
    "Guards & capture",
  );
  await expect(page.getByText("Not inspectable")).toHaveCount(2);
  await expect(page.getByText("Provider-bound requests")).toBeVisible();
  await page.getByRole("button", { name: "Review before enabling" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Enable local proxy capture?",
  });
  await expect(
    dialog.getByText(
      "Prompt and tool payload chunks needed for context attribution",
    ),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Not now" }).click();
  await expect(page.getByText("Proxy active")).toHaveCount(0);
});

test("analysis preview cancellation sends nothing", async ({ page }) => {
  await openCase(page, "analysis-preview-cancel", "/analysis", "AI analysis");
  const candidate = page
    .locator("article")
    .filter({ hasText: "analysis-73A9" });
  await candidate.getByRole("button", { name: "Analyze" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Review AI analysis egress",
  });
  await expect(
    dialog.getByText("analysis-73A9", { exact: true }).first(),
  ).toBeVisible();
  await expect(dialog.getByText("Synthetic local QA provider")).toBeVisible();
  await expect(dialog.getByText("fixture-summary-v1")).toBeVisible();
  expect((await fixtureAudit(page)).outboundAnalysisRequests).toBe(0);
  await dialog.getByRole("button", { name: "Cancel — send nothing" }).click();
  await expect(dialog).toHaveCount(0);
  expect((await fixtureAudit(page)).outboundAnalysisRequests).toBe(0);
  await expect(
    page.getByRole("heading", { name: "Analysis complete" }),
  ).toHaveCount(0);
});

test("writer-lock retry does not clear unrelated quarantine", async ({
  page,
}) => {
  await openCase(page, "degraded-health-recovery", "/health", "System health");
  await expect(page.getByText("Quarantine", { exact: true })).toBeVisible();
  await expect(page.getByText("spool:malformed-73A9")).toBeVisible();
  const writer = page.locator("article").filter({ hasText: "writer lock" });
  const quarantine = page.locator("article").filter({ hasText: "quarantine" });
  await expect(writer.getByText("degraded")).toBeVisible();
  await expect(quarantine.getByText("degraded")).toBeVisible();
  await writer.getByRole("button", { name: "Retry writer lock" }).click();
  await expect(writer.getByText("healthy")).toBeVisible();
  await expect(quarantine.getByText("degraded")).toBeVisible();
  await expect(page.getByText("spool:malformed-73A9")).toBeVisible();
});

test("central MCP toggle leaves plugin and search MCP unchanged", async ({
  page,
}) => {
  await openCase(page, "mcp-plugin-inventory", "/inventory", "Agent inventory");
  const docs = page.locator("article").filter({ hasText: "docs-73A9" });
  await docs.getByRole("switch").click();
  await expect(
    docs.getByRole("switch", {
      name: "Enable docs-73A9 for all configured harnesses",
    }),
  ).toBeVisible();
  await expect(page.getByText("Cutokyo search MCP")).toBeVisible();
  await page.getByRole("tab", { name: /Plugins/ }).click();
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

test("dashboard and consent surfaces pass browser accessibility checks", async ({
  page,
}) => {
  await openCase(page, "populated-dashboard", "/dashboard", "Overview");
  const dashboard = await new AxeBuilder({ page }).analyze();
  expect(dashboard.violations).toEqual([]);

  await openCase(page, "analysis-preview-cancel", "/analysis", "AI analysis");
  const candidate = page
    .locator("article")
    .filter({ hasText: "analysis-73A9" });
  await candidate.getByRole("button", { name: "Analyze" }).click();
  const consentDialog = page.getByRole("dialog", {
    name: "Review AI analysis egress",
  });
  await expect(consentDialog).toBeVisible();
  await expect(
    consentDialog.getByRole("button", {
      name: "Confirm and send redacted payload",
    }),
  ).toBeEnabled();
  const consent = await new AxeBuilder({ page }).analyze();
  expect(consent.violations).toEqual([]);
});

test("captures the seven representative states at all required desktop sizes", async ({
  page,
}) => {
  await mkdir(SCREENSHOTS, { recursive: true });
  const sizes = [
    { width: 900, height: 700 },
    { width: 1280, height: 800 },
    { width: 1536, height: 960 },
  ] as const;

  for (const size of sizes) {
    await page.setViewportSize(size);
    const suffix = `${size.width}x${size.height}`;

    await openCase(
      page,
      "onboarding-empty-history",
      "/onboarding",
      "See your agent work without sending it away",
    );
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `onboarding-${suffix}.png`),
    });

    await openCase(page, "empty-history", "/sessions", "Sessions");
    await expect(page.getByText("History is empty—not zero")).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `empty-history-${suffix}.png`),
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
      name: "Resume this exact native session?",
    });
    await expect(resume.getByRole("code")).toHaveText("claude-jev-73A9");
    await expect(
      resume.getByRole("button", { name: "Resume in Claude Code" }),
    ).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `session-resume-${suffix}.png`),
    });

    await openCase(
      page,
      "mcp-plugin-inventory",
      "/inventory",
      "Agent inventory",
    );
    await expect(page.getByText("docs-73A9")).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `inventory-mcp-${suffix}.png`),
    });

    await openCase(page, "analysis-preview-cancel", "/analysis", "AI analysis");
    await page
      .locator("article")
      .filter({ hasText: "analysis-73A9" })
      .getByRole("button", { name: "Analyze" })
      .click();
    const analysisConsent = page.getByRole("dialog", {
      name: "Review AI analysis egress",
    });
    await expect(analysisConsent).toBeVisible();
    await expect(
      analysisConsent.getByRole("button", {
        name: "Confirm and send redacted payload",
      }),
    ).toBeEnabled();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `analysis-consent-${suffix}.png`),
    });

    await openCase(
      page,
      "degraded-health-recovery",
      "/health",
      "System health",
    );
    await expect(page.getByText("spool:malformed-73A9")).toBeVisible();
    await expectNoWindowOverflow(page);
    await page.screenshot({
      path: resolve(SCREENSHOTS, `health-degraded-${suffix}.png`),
    });
  }
});
