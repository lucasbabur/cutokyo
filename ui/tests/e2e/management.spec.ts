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
  await page
    .getByRole("button", { name: "protocol-check", exact: true })
    .click();
  const manager = page.getByRole("complementary", { name: "protocol-check" });
  await expect(
    manager.getByRole("textbox", { name: "Source content" }),
  ).not.toHaveValue("");
  const harnesses = manager.getByRole("list", { name: "Harnesses" });
  await harnesses
    .getByRole("button", { name: "Install to Claude Code" })
    .click();
  const install = page.getByRole("dialog", {
    name: "Install protocol-check to Claude Code",
  });
  await expect(
    install.getByText("Supporting files are copied", { exact: false }),
  ).toBeVisible();
  await install.getByRole("button", { name: "Install to Claude Code" }).click();
  await expect(page.getByRole("dialog")).not.toBeVisible();
  const row = page.locator("tr.tool-row").filter({
    has: page.getByRole("button", { name: "protocol-check", exact: true }),
  });
  await expect(row).toHaveCount(1);
  await expect(
    row.getByRole("img", { name: "Installed in Claude Code" }),
  ).toBeVisible();
  await harnesses.getByRole("button", { name: /^Claude Code/ }).click();
  await manager.getByRole("button", { name: "Remove", exact: true }).click();
  const removal = page.getByRole("dialog", { name: "Remove protocol-check?" });
  await expect(removal.getByText("Claude Code", { exact: true })).toBeVisible();
  await removal.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(manager).toBeVisible();
  await manager.getByRole("button", { name: "Remove", exact: true }).click();
  await removal.getByRole("button", { name: "Remove installation" }).click();
  await expect(
    row.getByRole("img", { name: "Installed in Claude Code" }),
  ).toHaveCount(0);
  await expect(
    row.getByRole("img", { name: "Installed in OpenCode" }),
  ).toBeVisible();
});

test("shared instructions explain affected harnesses and editor passes accessibility", async ({
  page,
}) => {
  await page.goto(inventoryUrl);
  await page.getByRole("button", { name: "AGENTS.md", exact: true }).click();
  const manager = page.getByRole("complementary", { name: "AGENTS.md" });
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
  await manager.getByRole("button", { name: "Close details" }).click();
  await expect(
    page.getByRole("button", { name: "AGENTS.md", exact: true }),
  ).toBeFocused();
});

test("mouse clicks leave no focus ring; the keyboard brings it back", async ({
  page,
}) => {
  await page.goto(inventoryUrl);
  await page
    .getByRole("button", { name: "protocol-check", exact: true })
    .click();
  const tab = page.getByRole("tab", { name: "Details" });
  await tab.click();
  await expect(tab).toBeFocused();
  const outline = () =>
    tab.evaluate((node) => getComputedStyle(node).outlineStyle);
  expect(await outline()).toBe("none");
  await page.keyboard.press("ArrowLeft");
  await expect(page.getByRole("tab", { name: "File" })).toBeFocused();
  expect(
    await page
      .getByRole("tab", { name: "File" })
      .evaluate((node) => getComputedStyle(node).outlineStyle),
  ).toBe("solid");
});
