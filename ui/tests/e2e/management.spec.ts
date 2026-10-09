import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const inventoryUrl = "/?jev_case=mcp-plugin-inventory#/inventory";

test("skip link focuses the active page without changing its route", async ({
  page,
}) => {
  await page.goto("/?jev_case=visual-keyboard-consistency#/sessions");
  await expect(
    page.getByRole("heading", { name: "Sessions", level: 1 }),
  ).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("link", { name: "Skip to content" }),
  ).toBeFocused();
  const url = page.url();
  await page.keyboard.press("Enter");
  await expect(page.locator("#main-content")).toBeFocused();
  expect(page.url()).toBe(url);
  await expect(
    page.getByRole("heading", { name: "Sessions", level: 1 }),
  ).toBeVisible();
});

test("skill install and removal keep the independent original", async ({
  page,
}) => {
  await page.goto(inventoryUrl);
  await page.getByRole("button", { name: "Manage protocol-check" }).click();
  const manager = page.getByRole("dialog", { name: "Manage protocol-check" });
  await manager.getByRole("tab", { name: "File" }).click();
  await expect(
    manager.getByRole("textbox", { name: "Source content" }),
  ).not.toHaveValue("");
  await manager.getByRole("button", { name: /Install to…/ }).click();
  const targets = manager.getByRole("list", { name: "Install to a harness" });
  await expect(targets.locator(".quiet-badge--installed")).toHaveText(
    /Installed/,
  );
  await targets.getByRole("button", { name: "Install to Claude Code" }).click();
  const install = page.getByRole("dialog", {
    name: "Install protocol-check to Claude Code",
  });
  await expect(
    install.getByText("Supporting files are copied", { exact: false }),
  ).toBeVisible();
  await install
    .getByRole("button", { name: "Confirm install to Claude Code" })
    .click();
  await expect(page.getByRole("dialog")).not.toBeVisible();
  const copies = page.locator("article").filter({
    has: page.getByRole("button", { name: "Manage protocol-check" }),
  });
  await expect(copies).toHaveCount(2);
  const copy = copies.filter({ hasText: "Claude Code" });
  await copy.getByRole("button", { name: "Manage protocol-check" }).click();
  await manager.getByRole("tab", { name: "File" }).click();
  await manager.getByRole("button", { name: "Remove", exact: true }).click();
  const removal = page.getByRole("dialog", { name: "Remove protocol-check?" });
  await removal.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(manager).toBeVisible();
  await manager.getByRole("button", { name: "Remove", exact: true }).click();
  await removal.getByRole("button", { name: "Remove installation" }).click();
  await expect(copies).toHaveCount(1);
  await expect(copies.getByText("OpenCode", { exact: true })).toBeVisible();
});

test("shared instructions explain affected harnesses and editor passes accessibility", async ({
  page,
}) => {
  await page.goto(inventoryUrl);
  await page.getByRole("heading", { name: /^Instructions/ }).waitFor();
  await page.getByRole("button", { name: "Manage AGENTS.md" }).click();
  const manager = page.getByRole("dialog", { name: "Manage AGENTS.md" });
  await manager.getByRole("tab", { name: "File" }).click();
  await expect(
    manager.getByRole("textbox", { name: "Source content" }),
  ).not.toHaveValue("");
  const scan = await new AxeBuilder({ page }).analyze();
  expect(scan.violations).toEqual([]);
  await manager.getByRole("button", { name: "Remove", exact: true }).click();
  const removal = page.getByRole("dialog", { name: "Remove AGENTS.md?" });
  await expect(
    removal.getByText("Codex, OpenCode", { exact: true }),
  ).toBeVisible();
  await removal.getByRole("button", { name: "Cancel", exact: true }).click();
  await manager.getByRole("button", { name: "Done", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Manage AGENTS.md" }),
  ).toBeFocused();
});
