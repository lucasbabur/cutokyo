import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const screenshots = resolve(
  import.meta.dirname,
  "../../../evidence/management/screens",
);

for (const theme of ["light", "dark"] as const) {
  for (const viewport of [
    { width: 900, height: 700 },
    { width: 1280, height: 800 },
    { width: 1536, height: 960 },
  ]) {
    test(`management layout ${theme} ${viewport.width}x${viewport.height}`, async ({
      page,
    }) => {
      await mkdir(screenshots, { recursive: true });
      await page.setViewportSize(viewport);
      await page.emulateMedia({ colorScheme: theme });
      await page.goto("/?jev_case=mcp-plugin-inventory#/inventory");
      await expect(
        page.getByRole("heading", { name: "Agent tools", level: 1 }),
      ).toBeVisible();
      await expect(
        page.getByRole("button", { name: "docs-73A9", exact: true }),
      ).toBeInViewport();
      const capture = async (state: string) => {
        await page.evaluate(async () => {
          await document.fonts.ready;
          await Promise.all(
            document
              .getAnimations()
              .filter(
                (animation) =>
                  animation.effect?.getComputedTiming().iterations !== Infinity,
              )
              .map((animation) => animation.finished.catch(() => undefined)),
          );
        });
        const geometry = await page.evaluate(() => ({
          width: window.innerWidth,
          document: document.documentElement.scrollWidth,
        }));
        expect(geometry.document).toBeLessThanOrEqual(geometry.width);
        await page.screenshot({
          path: resolve(
            screenshots,
            `${state}-${theme}-${viewport.width}x${viewport.height}.png`,
          ),
        });
      };
      await capture("installations");
      await page
        .getByRole("button", { name: "protocol-check", exact: true })
        .click();
      const manager = page.getByRole("complementary", {
        name: "protocol-check",
      });
      await expect(
        manager.getByRole("textbox", { name: "Source content" }),
      ).not.toHaveValue("");
      const editor = manager.getByRole("textbox", { name: "Source content" });
      const original = await editor.inputValue();
      const save = manager.getByRole("button", { name: "Save changes" });
      await expect(save).toBeDisabled();
      await capture("tool-panel");
      await editor.fill("# edit\n");
      await expect(save).toBeEnabled();
      await expect(save).toBeInViewport();
      await capture("skill-editor");
      await editor.fill(original);
      await expect(save).toBeDisabled();
      await manager
        .getByRole("button", { name: "Install to Claude Code" })
        .click();
      const install = page.getByRole("dialog", {
        name: "Install protocol-check to Claude Code",
      });
      await expect(
        install.getByRole("button", {
          name: "Install to Claude Code",
        }),
      ).toBeInViewport();
      await capture("install-preview");
      await install
        .getByRole("button", { name: "Cancel", exact: true })
        .click();
      await manager
        .getByRole("button", { name: "Remove", exact: true })
        .click();
      const removal = page.getByRole("dialog", {
        name: "Remove protocol-check?",
      });
      await expect(
        removal.getByRole("button", { name: "Remove installation" }),
      ).toBeInViewport();
      await capture("remove-preview");
      const destructiveIcon = await removal
        .getByRole("button", { name: "Remove installation" })
        .locator("svg")
        .evaluate((icon) => {
          const reference = document.createElement("span");
          reference.hidden = true;
          reference.style.color = "var(--danger-icon)";
          document.body.append(reference);
          const expected = getComputedStyle(reference).color;
          reference.remove();
          return { actual: getComputedStyle(icon).color, expected };
        });
      expect(destructiveIcon.actual).toBe(destructiveIcon.expected);
    });

    test(`capture management layout ${theme} ${viewport.width}x${viewport.height}`, async ({
      page,
    }) => {
      await page.setViewportSize(viewport);
      await page.emulateMedia({ colorScheme: theme });
      await page.goto("/?jev_case=populated-dashboard#/settings");
      await expect(
        page.getByRole("heading", { name: "Settings", level: 1 }),
      ).toBeVisible();
      await page.getByRole("link", { name: "Manage capture" }).click();
      await expect(
        page.getByRole("heading", { name: "Manage capture", level: 1 }),
      ).toBeVisible();
      await expect(page.getByRole("button", { name: "Continue" })).toHaveCount(
        0,
      );
      await expect(
        page.getByRole("button", { name: "Skip for now" }),
      ).toHaveCount(0);
      for (const name of ["Claude Code", "Codex", "OpenCode"]) {
        const row = page.getByRole("listitem", { name: `${name} capture` });
        await expect(row.getByRole("checkbox")).toHaveCount(0);
        await row
          .getByRole("button", { name: `More actions for ${name}` })
          .click();
        await row.getByRole("menuitem", { name: `Install ${name}` }).click();
        await page.getByRole("button", { name: "Confirm" }).click();
        await expect(row.getByText("Ready")).toBeVisible();
      }
      const rowBoxes = await page.locator(".harness-row").evaluateAll((rows) =>
        rows.map((row) => {
          const box = row.getBoundingClientRect();
          return { width: box.width, height: box.height };
        }),
      );
      for (const box of rowBoxes) {
        expect(box.width).toBeGreaterThan(300);
        expect(Math.abs(box.height - rowBoxes[0]!.height)).toBeLessThanOrEqual(
          1,
        );
      }
      const overflow = await page.evaluate(() => ({
        width: window.innerWidth,
        document: document.documentElement.scrollWidth,
      }));
      expect(overflow.document).toBeLessThanOrEqual(overflow.width);
      await page
        .getByRole("heading", { name: "Manage capture", level: 1 })
        .scrollIntoViewIfNeeded();
      await mkdir(screenshots, { recursive: true });
      await page.screenshot({
        path: resolve(
          screenshots,
          `capture-management-${theme}-${viewport.width}x${viewport.height}.png`,
        ),
      });
    });
  }
}
