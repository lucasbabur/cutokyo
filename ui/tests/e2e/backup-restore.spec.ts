import { resolve } from "node:path";

import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

for (const theme of ["light", "dark"] as const) {
  for (const [width, height] of [
    [900, 700],
    [1280, 800],
    [1536, 960],
  ] as const) {
    test(`local backup and restore at ${width}x${height} in ${theme}`, async ({
      page,
    }, testInfo) => {
      const capture = async (state: string) => {
        const name = `${state}-${theme}-${width}x${height}`;
        const path = resolve(
          "../evidence/management/backup/screens",
          `${name}.png`,
        );
        await page.screenshot({ path });
        await testInfo.attach(name, { path, contentType: "image/png" });
      };
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto("/?jev_case=retention-delete#/data");
      await expect(
        page.getByRole("heading", { name: "Data controls", level: 1 }),
      ).toBeVisible();
      const stored = await page.getByText(/\d+ sessions? stored/).innerText();
      const initialSessions = Number(/(\d+)/.exec(stored)?.[1]);
      expect(initialSessions).toBeGreaterThan(0);
      await page
        .getByRole("button", { name: "Create backup", exact: true })
        .click();
      let dialog = page.getByRole("dialog", { name: "Create a local backup" });
      await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
      await expect(page.getByRole("dialog")).not.toBeVisible();
      await expect(page.getByText(/No backups yet/)).toBeVisible();
      await page
        .getByRole("button", { name: "Create backup", exact: true })
        .click();
      dialog = page.getByRole("dialog", { name: "Create a local backup" });
      await dialog
        .getByRole("button", { name: "Create backup", exact: true })
        .click();
      dialog = page.getByRole("dialog", { name: "Backup created" });
      await expect(
        dialog.getByText(/\/isolated-cutokyo\/backups\/backup-/),
      ).toBeVisible();
      const createdScan = await new AxeBuilder({ page }).analyze();
      expect(createdScan.violations).toEqual([]);
      await capture("backup-created");
      await dialog.getByRole("button", { name: "Done", exact: true }).click();
      await page.getByRole("button", { name: "Restore…", exact: true }).click();
      dialog = page.getByRole("dialog", { name: "Restore this backup?" });
      await expect(
        dialog.getByRole("button", { name: "Restore selected backup" }),
      ).toBeEnabled();
      await dialog
        .getByRole("button", { name: "Cancel — keep current history" })
        .click();
      await expect(
        page.getByText(`${initialSessions} sessions stored`),
      ).toBeVisible();
      await page.getByRole("button", { name: "Delete all…" }).click();
      const deletion = page.getByRole("dialog", {
        name: "Delete all local history?",
      });
      await deletion.getByRole("textbox").fill("DELETE ALL LOCAL HISTORY");
      await deletion
        .getByRole("button", { name: "Delete all local history", exact: true })
        .click();
      await expect(page.getByText(/0 sessions stored/)).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Delete all…" }),
      ).toBeDisabled();
      await page.getByRole("button", { name: "Restore…", exact: true }).click();
      dialog = page.getByRole("dialog", { name: "Restore this backup?" });
      await expect(
        dialog.getByText(
          new RegExp(
            `0 current sessions will be replaced by ${initialSessions}`,
          ),
        ),
      ).toBeVisible();
      const previewScan = await new AxeBuilder({ page }).analyze();
      expect(previewScan.violations).toEqual([]);
      await capture("backup-restore-confirmation");
      await dialog
        .getByRole("button", { name: "Restore selected backup" })
        .click();
      dialog = page.getByRole("dialog", { name: "History restored" });
      await expect(
        dialog.getByText("Recovery copy", { exact: true }),
      ).toBeVisible();
      await expect(dialog.getByText("ok", { exact: true })).toBeVisible();
      await capture("backup-restored");
      await dialog.getByRole("button", { name: "Done", exact: true }).click();
      await expect(
        page.getByText(`${initialSessions} sessions stored`),
      ).toBeVisible();
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth > window.innerWidth,
      );
      expect(overflow).toBe(false);
    });
  }
}
