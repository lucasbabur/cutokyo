import { expect, test, type Page } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const SEARCH_SCREENS = resolve(
  import.meta.dirname,
  "../../../evidence/management/search",
);

async function settleCapture(page: Page) {
  await page.evaluate(
    () =>
      new Promise<void>((resolve, reject) => {
        const timeout = window.setTimeout(
          () =>
            reject(new Error("Search capture did not settle within 3 seconds")),
          3_000,
        );
        const ready = async () => {
          await document.fonts.ready;
          await Promise.all(
            document
              .getAnimations()
              .filter((animation) =>
                Number.isFinite(animation.effect?.getComputedTiming().endTime),
              )
              .map((animation) => animation.finished.catch(() => undefined)),
          );
          await new Promise<void>((painted) =>
            requestAnimationFrame(() => requestAnimationFrame(() => painted())),
          );
        };
        void ready().then(
          () => {
            window.clearTimeout(timeout);
            resolve();
          },
          (reason) => {
            window.clearTimeout(timeout);
            reject(reason);
          },
        );
      }),
  );
}

test("matching-source previews fit the compact reference in both themes and all desktop widths", async ({
  page,
}) => {
  await mkdir(SEARCH_SCREENS, { recursive: true });
  await page.goto("/?jev_case=search-detail-resume#/sessions");
  await page
    .getByRole("searchbox", { name: "Search session content" })
    .fill("JEV 73A9 capture");
  await expect(page.getByText("1 session", { exact: true })).toBeVisible();
  for (const theme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: theme });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    for (const size of [
      { width: 900, height: 700 },
      { width: 1280, height: 800 },
      { width: 1536, height: 960 },
    ]) {
      await page.setViewportSize(size);
      await expect(page.locator(".session-matches")).toBeVisible();
      const geometry = await page.evaluate(() => ({
        width: innerWidth,
        documentWidth: document.documentElement.scrollWidth,
        bodyWidth: document.body.scrollWidth,
      }));
      expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.width);
      expect(geometry.bodyWidth).toBeLessThanOrEqual(geometry.width);
      await settleCapture(page);
      await page.screenshot({
        path: resolve(
          SEARCH_SCREENS,
          `search-matches-${theme}-${size.width}x${size.height}.png`,
        ),
      });
    }
  }
});

test("collapsed additional filters name exact retained constraints and fit 900px", async ({
  page,
}) => {
  await mkdir(SEARCH_SCREENS, { recursive: true });
  await page.setViewportSize({ width: 900, height: 700 });
  await page.goto("/?jev_case=search-pagination#/sessions");
  const project = `/work/${"very-long-project-directory-".repeat(12)}`;
  await page.evaluate((value) => {
    sessionStorage.setItem(
      `cutokyo.session-search:${location.search}`,
      JSON.stringify({
        project: value,
        branch: "feature/observability",
        tool: "Read",
        skill: "search-review",
        agent: "archive-agent",
      }),
    );
  }, project);
  await page.reload();
  const summary = page.getByLabel("Active additional filters");
  await expect(summary).toBeVisible();
  await expect(page.locator(".advanced-filters")).not.toHaveAttribute("open");
  for (const text of [
    `Project: ${project}`,
    "Branch: feature/observability",
    "Tool: Read",
    "Skill: search-review",
    "Agent: archive-agent",
  ]) {
    await expect(summary).toContainText(text);
  }
  for (const theme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: theme });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const geometry = await page.evaluate(() => ({
      width: innerWidth,
      documentWidth: document.documentElement.scrollWidth,
      bodyWidth: document.body.scrollWidth,
    }));
    expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.width);
    expect(geometry.bodyWidth).toBeLessThanOrEqual(geometry.width);
    await expect
      .poll(() =>
        summary.evaluate((node) => {
          const probe = document.createElement("span");
          probe.style.color = getComputedStyle(node)
            .getPropertyValue("--text-secondary")
            .trim();
          node.append(probe);
          const expectedColor = getComputedStyle(probe).color;
          probe.remove();
          return [node, ...node.querySelectorAll("span, strong")].every(
            (element) => getComputedStyle(element).color === expectedColor,
          );
        }),
      )
      .toBe(true);
    await settleCapture(page);
    await page.screenshot({
      path: resolve(
        SEARCH_SCREENS,
        `search-active-filters-${theme}-900x700.png`,
      ),
    });
  }
});

test("literal terms, explicit phrases, metadata previews and empty recovery use real fixture state", async ({
  page,
}) => {
  await page.goto("/?jev_case=search-detail-resume#/sessions");
  const query = page.getByRole("searchbox", { name: "Search session content" });
  await query.fill("JEV 73A9 capture");
  await expect(page.getByText("1 session", { exact: true })).toBeVisible();
  await expect(
    page.locator(".session-matches mark").filter({ hasText: "73A9" }).first(),
  ).toBeVisible();
  await page.getByLabel("Match", { exact: true }).selectOption("phrase");
  await expect(page.getByText("No sessions match these filters")).toBeVisible();
  await page.getByLabel("Match", { exact: true }).selectOption("terms");
  await query.fill("claude native 73A9");
  await expect(page.getByText("1 session", { exact: true })).toBeVisible();
  await expect(
    page.locator(".session-match-source").filter({ hasText: "Session ID" }),
  ).toBeVisible();
  await query.fill('*: " ()');
  await expect(page.getByText("No sessions match these filters")).toBeVisible();
  await expect(
    page.getByText("Session search is unavailable"),
  ).not.toBeVisible();
  await page.getByRole("button", { name: "Clear search and filters" }).click();
  await expect(query).toHaveValue("");
  await expect(page.getByText("3 sessions", { exact: true })).toBeVisible();
});

test("relevance differs from newest and actual late-page facets filter retained history", async ({
  page,
}) => {
  await page.goto("/?jev_case=search-pagination#/sessions");
  await page
    .getByRole("searchbox", { name: "Search session content" })
    .fill("needle");
  await expect(page.getByText("57 sessions", { exact: true })).toBeVisible();
  await expect(
    page.locator(".session-list .session-row").first(),
  ).toContainText("Archive needle");
  await page.getByLabel("Sort", { exact: true }).selectOption("newest");
  await expect(
    page.locator(".session-list .session-row").first(),
  ).toContainText("Archive work 56");
  await page.getByText("More filters", { exact: false }).click();
  await expect(
    page
      .getByLabel("Tool", { exact: true })
      .locator("option", { hasText: "LateTool" }),
  ).toHaveCount(1);
  await page.getByLabel("Tool", { exact: true }).selectOption("LateTool");
  await expect(page.getByText("1 session", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Archive work 56", exact: true }),
  ).toBeVisible();
  await page.getByLabel("Tool", { exact: true }).selectOption("");
  await expect(page.getByText("57 sessions", { exact: true })).toBeVisible();
});

test("exact detail and back navigation retain query, filters, sort and the second page", async ({
  page,
}) => {
  await page.goto("/?jev_case=search-pagination#/sessions");
  const query = page.getByRole("searchbox", { name: "Search session content" });
  await query.fill("archive needle");
  await page.getByRole("button", { name: "Claude Code", exact: true }).click();
  await page.getByLabel("Sort", { exact: true }).selectOption("newest");
  await expect(page.getByText("57 sessions", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Next page", exact: true }).click();
  await expect(page.getByText("51–57 of 57", { exact: true })).toBeVisible();
  await page.getByRole("link", { name: "Archive needle", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Timeline" })).toBeVisible();
  await page.getByRole("link", { name: "Back to sessions" }).click();
  await expect(query).toHaveValue("archive needle");
  await expect(
    page.getByRole("button", { name: "Claude Code", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByLabel("Sort", { exact: true })).toHaveValue("newest");
  await expect(page.getByText("51–57 of 57", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Archive needle", exact: true }),
  ).toBeVisible();
});
