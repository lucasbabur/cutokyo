import { strict as assert } from "node:assert";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { $, $$, browser } from "@wdio/globals";

const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
assert.ok(
  root,
  "native inventory tests require the isolated package-runner root",
);
const home = join(root, "data/native-harness-fixture");
const evidence = process.env.CUTOKYO_NATIVE_EVIDENCE;
assert.ok(evidence, "native inventory evidence directory was not propagated");
const sourceSkill = join(home, ".claude/skills/release-checklist");
const copiedSkill = join(home, ".codex/skills/release-checklist");
const claudeConfig = join(home, ".claude.json");

async function tools() {
  await $("a=Agent tools").click();
  await $("h1=Agent tools").waitForDisplayed();
}
const panel = (name: string) => $(`aside[aria-label="${name}"]`);
async function manage(name: string) {
  await $(`tr button[aria-label="${name}"]`).click();
  await panel(name).waitForDisplayed();
  await $('textarea[aria-label="Source content"]').waitForDisplayed();
}
async function waitForDialogClose() {
  await $('[role="dialog"]').waitForExist({ reverse: true });
}
async function waitForReceipt() {
  await $(".success-message").waitForDisplayed();
}
async function filterHarness(name: string) {
  const filter = await $('nav[aria-label="Tool filters"]').$(`button*=${name}`);
  await filter.click();
  assert.equal(await filter.getAttribute("aria-pressed"), "true");
}
async function installTo(name: string) {
  await $(
    `ul[aria-label="Harnesses"] button[aria-label="Install to ${name}"]`,
  ).click();
  const confirm = await $('[role="dialog"]').$(`button*=Install to ${name}`);
  await confirm.waitForDisplayed();
  assert.equal(await confirm.isEnabled(), true);
  await confirm.click();
}
/** Shows the installation that lives in `harness` when a tool has several. */
async function showInstallation(harness: string) {
  const row = await $('ul[aria-label="Harnesses"]').$(`button*=${harness}`);
  if (await row.isExisting()) await row.click();
}

describe("Native inventory management on real isolated harness files", () => {
  if (process.env.CUTOKYO_NATIVE_PHASE === "save") {
    it("edits, copies all skill assets, then removes only the selected native bundle", async () => {
      await tools();
      await $('input[aria-label="Search installed tools"]').setValue(
        "release-checklist",
      );
      await manage("release-checklist");
      const source = await $('textarea[aria-label="Source content"]');
      assert.match(await source.getValue(), /Run the tests before shipping/);
      const edited =
        "---\nname: release-checklist\ndescription: Native edited release checklist\n---\n# Native edited checklist\nVerify the actual release.\n";
      await source.setValue(edited);
      await $("button*=Save changes").click();
      await waitForReceipt();
      assert.equal(readFileSync(join(sourceSkill, "SKILL.md"), "utf8"), edited);

      await manage("release-checklist");
      await installTo("Codex");
      await waitForDialogClose();
      assert.equal(readFileSync(join(copiedSkill, "SKILL.md"), "utf8"), edited);
      assert.equal(
        readFileSync(join(copiedSkill, "references/checks.md"), "utf8"),
        readFileSync(join(sourceSkill, "references/checks.md"), "utf8"),
      );

      await filterHarness("Claude Code");
      await manage("release-checklist");
      await showInstallation("Claude Code");
      await panel("release-checklist").$("button=Details").click();
      assert.ok(
        (await panel("release-checklist").getText()).includes(sourceSkill),
      );
      // An independent copy now exists, so no harness offers another install.
      assert.equal(
        (
          await $$(
            'ul[aria-label="Harnesses"] button[aria-label^="Install to"]',
          )
        ).length,
        0,
      );
      await panel("release-checklist").$("button=File").click();
      await $("button*=Remove").click();
      assert.match(await $('[role="dialog"]').getText(), /recovery copy/i);
      await browser.saveScreenshot(
        join(evidence, "inventory-remove-confirmation-1280x800.png"),
      );
      await $("button=Remove installation").click();
      await waitForDialogClose();
      assert.equal(existsSync(sourceSkill), false);
      assert.equal(existsSync(join(copiedSkill, "references/checks.md")), true);
      await $('button[aria-label="Manage release-checklist"]').waitForExist({
        reverse: true,
      });
      assert.ok(readdirSync(join(root, "data/inventory-recovery")).length >= 3);
    });

    it("rejects stale and malformed MCP edits, converts real config credentials, and protects capture hooks", async () => {
      await tools();
      await filterHarness("Claude Code");
      await $('input[aria-label="Search installed tools"]').setValue(
        "project-notes",
      );
      await manage("project-notes");
      await panel("project-notes").$("button=Details").click();
      assert.ok(
        (await panel("project-notes").getText()).includes(claudeConfig),
      );
      await panel("project-notes").$("button=File").click();
      const editor = await $('textarea[aria-label="Source content"]');
      const invalid = "{not valid JSON";
      const before = readFileSync(claudeConfig, "utf8");
      await editor.setValue(invalid);
      await $("button*=Save changes").click();
      await $('.form-error[role="alert"]').waitForDisplayed();
      assert.equal(await editor.getValue(), invalid);
      assert.equal(readFileSync(claudeConfig, "utf8"), before);
      assert.match(
        await $('.form-error[role="alert"]').getText(),
        /syntax|JSON/i,
      );

      const edited = JSON.stringify(
        {
          command: "node",
          args: ["changed-notes.js"],
          env: { NOTES_TOKEN: "isolated-test-token" },
        },
        null,
        2,
      );
      await editor.setValue(edited);
      const external = JSON.parse(before);
      external.unrelated = "concurrent user edit";
      writeFileSync(claudeConfig, JSON.stringify(external));
      await $("button*=Save changes").click();
      await browser.waitUntil(async () =>
        /changed since/.test(await $('.form-error[role="alert"]').getText()),
      );
      assert.equal(await editor.getValue(), edited);
      assert.equal(
        JSON.parse(readFileSync(claudeConfig, "utf8")).mcpServers[
          "project-notes"
        ].args[0],
        "notes.js",
      );
      await $('button[aria-label="Close details"]').click();
      await $("button=Discard changes").click();
      await waitForDialogClose();
      await $('button[aria-label="Rescan installations"]').click();
      await manage("project-notes");
      await $('textarea[aria-label="Source content"]').setValue(edited);
      await $("button*=Save changes").click();
      await waitForReceipt();
      assert.equal(
        JSON.parse(readFileSync(claudeConfig, "utf8")).unrelated,
        "concurrent user edit",
      );
      assert.equal(
        JSON.parse(readFileSync(claudeConfig, "utf8")).mcpServers[
          "project-notes"
        ].args[0],
        "changed-notes.js",
      );

      await manage("project-notes");
      await installTo("OpenCode");
      await waitForDialogClose();
      const openConfig = readFileSync(
        join(home, ".config/opencode/opencode.jsonc"),
        "utf8",
      );
      assert.match(openConfig, /Preserve this native comment/);
      assert.match(openConfig, /environment/);
      assert.match(openConfig, /isolated-test-token/);
      assert.match(openConfig, /changed-notes.js/);

      await $('input[aria-label="Search installed tools"]').setValue(
        "PostToolUse · cutokyo",
      );
      await manage("PostToolUse · cutokyo");
      const protectedSource = await $('textarea[aria-label="Source content"]');
      assert.notEqual(await protectedSource.getAttribute("readonly"), null);
      assert.equal(await protectedSource.getProperty("readOnly"), true);
      assert.match(
        await panel("PostToolUse · cutokyo").getText(),
        /Cutokyo-owned capture/,
      );
      assert.equal(await $("button*=Save changes").isExisting(), false);
      assert.equal(await $("button*=Remove").isExisting(), false);
      await $('button[aria-label="Close details"]').click();
      await browser.saveScreenshot(
        join(evidence, "inventory-native-configured-1280x800.png"),
      );
    });
  } else if (process.env.CUTOKYO_NATIVE_PHASE === "reopen") {
    it("rediscovers persisted native edits/copies without resurrecting deleted bundles", async () => {
      assert.equal(existsSync(sourceSkill), false);
      assert.match(
        readFileSync(join(copiedSkill, "SKILL.md"), "utf8"),
        /Native edited checklist/,
      );
      assert.equal(
        JSON.parse(readFileSync(claudeConfig, "utf8")).unrelated,
        "concurrent user edit",
      );
      await tools();
      await filterHarness("Codex");
      await $('input[aria-label="Search installed tools"]').setValue(
        "release-checklist",
      );
      await manage("release-checklist");
      await panel("release-checklist").$("button=Details").click();
      assert.ok(
        (await panel("release-checklist").getText()).includes(copiedSkill),
      );
      await panel("release-checklist").$("button=File").click();
      assert.match(
        await $('textarea[aria-label="Source content"]').getValue(),
        /Native edited checklist/,
      );
      await $('button[aria-label="Close details"]').click();
    });
  } else {
    throw new Error(
      "The isolated native runner must choose save or reopen phase",
    );
  }
});
