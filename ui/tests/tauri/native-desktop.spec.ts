import { strict as assert } from "node:assert";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { $, browser } from "@wdio/globals";

import { assertNativeGeometry } from "./geometry.js";

const nativeEvidence = process.env.CUTOKYO_NATIVE_EVIDENCE;
assert.ok(nativeEvidence, "native evidence directory was not propagated");
const evidencePath = join(nativeEvidence, "search-detail-resume-1280x800.png");

async function closeCompletedResumeTerminal(project: string, nativeId: string) {
  assert.ok(nativeEvidence, "native evidence directory was not propagated");
  const runner = fileURLToPath(
    new URL("../../scripts/native-e2e.py", import.meta.url),
  );
  const inspect = () =>
    execFileSync(
      "python3",
      [runner, "--inspect-resume-terminal", project, nativeId],
      {
        env: process.env,
        encoding: "utf8",
        timeout: 15_000,
      },
    );
  let proof = "";
  await browser.waitUntil(() => {
    proof = inspect();
    return JSON.parse(proof).ready === true;
  });
  const phase = process.env.CUTOKYO_NATIVE_PHASE;
  assert.ok(phase === "save" || phase === "capture-history");
  writeFileSync(
    join(nativeEvidence, `native-resume-terminal-before-${phase}.json`),
    proof,
    { flag: "wx" },
  );
  const receipt = execFileSync(
    "python3",
    [runner, "--cleanup-resume-terminal", project, nativeId, proof],
    { env: process.env, encoding: "utf8", timeout: 15_000 },
  );
  const result = JSON.parse(receipt);
  assert.equal(result.verdict, "PASS");
  assert.equal(result.terminalExited, true);
  assert.equal(result.descendantsExited, true);
  assert.equal(result.windowRemoved, true);
  assert.equal(result.applicationPreserved, true);
  assert.deepEqual(result.keyboardDispatches, []);
  assert.equal(result.focusChangedByHelper, false);
  writeFileSync(
    join(nativeEvidence, `native-resume-terminal-cleanup-${phase}.json`),
    receipt,
    { flag: "wx" },
  );
}

const row = (name: string) => $(`li[aria-label="${name} capture"]`);

async function chooseAction(name: string, action: string) {
  await $(`button[aria-label="More actions for ${name}"]`).click();
  await $(`button=${action} ${name}`).click();
}

async function dialogButton(label: string) {
  const dialog = await $('[role="dialog"]');
  await dialog.waitForDisplayed();
  return dialog.$(`button=${label}`);
}

async function waitForDialogGone() {
  await $('[role="dialog"]').waitForExist({ reverse: true });
}

async function expectReady(name: string, ready: boolean) {
  await browser.waitUntil(
    async () => (await row(name).getText()).includes("Ready") === ready,
    { timeoutMsg: `${name} Ready state should be ${ready}` },
  );
}

describe("Cutokyo native Tauri application", () => {
  if (process.env.CUTOKYO_NATIVE_PHASE === "save") {
    it("searches, opens, exactly resumes, and deletes an isolated native fixture", async () => {
      const applicationUrl = new URL(await browser.getUrl());
      assert.equal(applicationUrl.protocol, "tauri:");
      assert.equal(applicationUrl.hostname, "localhost");

      // Embedded WDIO sessions share the packaged application. Arrange this
      // journey's route explicitly instead of inheriting the previous spec's UI.
      await browser.execute(() => {
        globalThis.location.hash = "/dashboard";
      });
      const overview = await $("h1=Overview");
      await overview.waitForDisplayed();
      assert.equal(await overview.getText(), "Overview");

      const windowSize = (await browser.tauri.execute(({ core }) =>
        core.invoke("plugin:window|inner_size", { label: "main" }),
      )) as { height: number; width: number };
      const viewport = await browser.execute(() => ({
        width: window.innerWidth,
        height: window.innerHeight,
      }));
      assertNativeGeometry(windowSize, viewport);
      writeFileSync(
        join(nativeEvidence, "geometry-1280x800.json"),
        JSON.stringify({ windowSize, viewport }),
        { flag: "wx" },
      );

      const sessionsNavigation = await $("a=Sessions");
      await sessionsNavigation.click();
      const sessionsHeading = await $("h1=Sessions");
      await sessionsHeading.waitForDisplayed();

      const search = await $('input[aria-label="Search session content"]');
      await search.setValue("JEV exact resume needle 73A9");
      const resultLink = await $(
        'a[aria-label="Open Native exact resume 73A9"]',
      );
      await resultLink.waitForDisplayed();
      await resultLink.click();

      const detailHeading = await $("h1=Native exact resume 73A9");
      await detailHeading.waitForDisplayed();
      assert.equal(
        await $('main code[title="claude-native-73A9"]').getAttribute("title"),
        "claude-native-73A9",
      );

      await $("button=Resume").click();
      const resumeDialog = await $('[role="dialog"]');
      await resumeDialog.waitForDisplayed();
      await resumeDialog.$("p*=Folder:").waitForDisplayed();
      await browser.saveScreenshot(evidencePath);

      const confirmation = await $('[role="dialog"]').$("button=Resume");
      await confirmation.waitForEnabled();
      assert.equal(await confirmation.isEnabled(), true);
      await confirmation.click();
      // The shared app's global announcer can retain another action's status.
      // Wait for the session-detail receipt rendered after native acknowledgement.
      const success = await $('main .success-message[role="status"]');
      await success.waitForDisplayed();
      assert.match(
        await success.getText(),
        /started in a visible terminal with exact native ID claude-native-73A9/,
      );

      assert.match(
        await success.getText(),
        /cannot verify native session activation/,
      );
      const auditPath = process.env.CUTOKYO_NATIVE_RESUME_AUDIT;
      assert.ok(auditPath, "native resume audit path was not propagated");
      await browser.waitUntil(() => existsSync(auditPath), {
        timeout: 10_000,
        timeoutMsg: "the credential-free native harness was not launched",
      });
      assert.equal(
        readFileSync(auditPath, "utf8"),
        "--resume\nclaude-native-73A9\n",
      );
      const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
      assert.ok(root, "native test root was not propagated");
      const contextPath = auditPath.replace(/\.txt$/, ".context.json");
      await browser.waitUntil(() => existsSync(contextPath), {
        timeout: 10_000,
        timeoutMsg: "the interactive harness did not record its real context",
      });
      assert.deepEqual(JSON.parse(readFileSync(contextPath, "utf8")), {
        cwd: join(root, "data/native-resume-project"),
        stdinTty: true,
        stdoutTty: true,
      });

      await closeCompletedResumeTerminal(
        join(root, "data/native-resume-project"),
        "claude-native-73A9",
      );

      await $("button=Delete session").click();
      const deletionDialog = await $('[role="dialog"]');
      await deletionDialog.waitForDisplayed();
      const deletedSession = await deletionDialog.$(
        ".//dt[normalize-space(.)='Session']/following-sibling::dd[1]",
      );
      const messageCount = await deletionDialog.$(
        ".//dt[normalize-space(.)='Messages']/following-sibling::dd[1]",
      );
      await messageCount.waitForDisplayed();
      assert.equal(await deletedSession.getText(), "Native exact resume 73A9");
      assert.equal(await messageCount.getText(), "1");
      await $("button=Delete selected session").click();

      const returnedSessionsHeading = await $("h1=Sessions");
      await returnedSessionsHeading.waitForDisplayed();
      const retainedSearch = await $(
        'input[aria-label="Search session content"]',
      );
      assert.equal(
        await retainedSearch.getValue(),
        "JEV exact resume needle 73A9",
      );
      await $("h2=No sessions match these filters").waitForDisplayed();
      assert.equal(
        await $('[role="group"][aria-label="Harness"]')
          .$("button=All")
          .getAttribute("aria-pressed"),
        "true",
      );
      await browser.waitUntil(async () => {
        return (
          (await $(".results-heading strong").getText()) === "0 sessions" &&
          (await $('select[aria-label="Sort"]').getValue()) === "relevance" &&
          !(await $(".results-heading").getText()).includes(
            "Updating results",
          ) &&
          !(await $(".search-stale").isExisting())
        );
      });
      const deletedResult = await $(
        'a[aria-label="Open Native exact resume 73A9"]',
      );
      await deletedResult.waitForExist({ reverse: true });

      // Keep query continuity above, then explicitly recover to all history.
      // A filtered zero count alone cannot prove the retained store is empty.
      await $("button=Clear search and filters").click();
      assert.equal(await retainedSearch.getValue(), "");
      for (const [label, value] of [
        ["Match", "terms"],
        ["Sort", "relevance"],
        ["Date", "all"],
        ["Project", ""],
        ["Branch", ""],
        ["Tool", ""],
        ["Skill", ""],
        ["Agent", ""],
      ]) {
        assert.equal(
          await $(`select[aria-label="${label}"]`).getValue(),
          value,
        );
      }
      await browser.waitUntil(async () => {
        return (
          (await $(".results-heading strong").getText()) === "0 sessions" &&
          !(await $(".results-heading").getText()).includes(
            "Updating results",
          ) &&
          !(await $(".search-stale").isExisting())
        );
      });
      await deletedResult.waitForExist({ reverse: true });
      const emptyState = await $("h2=No sessions yet");
      await emptyState.waitForDisplayed();
    });

    it("saves appearance to native desktop state", async () => {
      const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
      assert.ok(root, "native test root was not propagated");
      const statePath = join(root, "data/desktop-state.json");

      await $("a=Settings").click();
      await $("h1=Settings").waitForDisplayed();
      await $("button=Dark").click();
      await browser.waitUntil(
        async () =>
          (await browser.execute(
            () => document.documentElement.dataset.theme,
          )) === "dark",
      );
      await browser.waitUntil(
        () =>
          existsSync(statePath) &&
          JSON.parse(readFileSync(statePath, "utf8")).appearance === "dark",
        { timeoutMsg: "the appearance preference was not saved to disk" },
      );
      await browser.saveScreenshot(
        join(nativeEvidence, "appearance-dark-1280x800.png"),
      );
    });
  } else if (process.env.CUTOKYO_NATIVE_PHASE === "reopen") {
    it("restores saved appearance after a native application restart", async () => {
      const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
      assert.ok(root, "native test root was not propagated");
      const statePath = join(root, "data/desktop-state.json");
      assert.equal(
        JSON.parse(readFileSync(statePath, "utf8")).appearance,
        "dark",
      );
      await $("a=Settings").click();
      await $("h1=Settings").waitForDisplayed();
      assert.equal(
        await browser.execute(() => document.documentElement.dataset.theme),
        "dark",
      );
      assert.equal(await $("button=Dark").getAttribute("aria-pressed"), "true");
      await $("button=System").click();
      await browser.waitUntil(
        () =>
          JSON.parse(readFileSync(statePath, "utf8")).appearance === "system",
        {
          timeoutMsg: "the system appearance preference was not saved to disk",
        },
      );
    });
  } else if (process.env.CUTOKYO_NATIVE_PHASE === "browse") {
    it("browses a real packaged application without installing native capture", async () => {
      const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
      assert.ok(root);
      await $("h1=Set up Cutokyo").waitForDisplayed();
      const browse = await $("button=Skip for now");
      assert.equal(await browse.isEnabled(), false);
      await $("//label[contains(., 'stored unencrypted')]/input").click();
      await browse.waitForEnabled();
      await browse.click();
      await $("h1=Overview").waitForDisplayed();
      const state = JSON.parse(
        readFileSync(join(root, "data/desktop-state.json"), "utf8"),
      );
      assert.equal(state.onboarding_complete, true);
      assert.deepEqual(state.selected_harnesses, []);
      assert.equal(existsSync(join(root, "data/capture-setup")), false);
      for (const target of [
        "home/.claude/settings.json",
        "home/.codex/hooks.json",
        "config/opencode/plugins/cutokyo.ts",
      ]) {
        assert.equal(
          existsSync(join(root, target)),
          false,
          "browse must not create native capture configuration",
        );
      }
      const proxy = (await browser.tauri.execute(({ core }) =>
        core.invoke("proxy_status"),
      )) as { proxyEnabled: boolean };
      assert.equal(proxy.proxyEnabled, false);
      await $("a=Sessions").click();
      await $("h2=No sessions yet").waitForDisplayed();
      await browser.saveScreenshot(
        join(nativeEvidence, "onboarding-browse-1280x800.png"),
      );
    });
  } else if (process.env.CUTOKYO_NATIVE_PHASE === "state-recovery") {
    it("preserves rejected desktop preferences and clears their single warning after explicit repair", async () => {
      const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
      assert.ok(root);
      const statePath = join(root, "data/desktop-state.json");
      const original = readFileSync(statePath, "utf8");
      const saved = JSON.parse(original);
      assert.equal(saved.onboarding_complete, true);
      assert.equal(saved.appearance, "dark");
      assert.deepEqual(saved.selected_harnesses, [
        "claude_code",
        "codex",
        "opencode",
      ]);
      assert.deepEqual(saved.mcp_enabled, {});
      await $("h1=Set up Cutokyo").waitForDisplayed();
      await $("button*=Warnings").click();
      const warning = await $(".warnings__panel");
      await warning.waitForDisplayed();
      assert.match(
        await warning.getText(),
        /Desktop settings could not be decoded/,
      );
      assert.match(await warning.getText(), /unknown field `mcp_enabled`/);
      const occurrences = await browser.execute(
        () =>
          document.body.textContent?.match(
            /Desktop settings could not be decoded/g,
          )?.length ?? 0,
      );
      assert.equal(
        occurrences,
        1,
        "the warnings panel must list the decode warning once and pages must not repeat it",
      );
      assert.equal(readFileSync(statePath, "utf8"), original);
      await browser.saveScreenshot(
        join(nativeEvidence, "state-warning-before-repair-1280x800.png"),
      );

      delete saved.mcp_enabled;
      const repaired = JSON.stringify(saved, null, 2) + "\n";
      writeFileSync(statePath, repaired, { mode: 0o600 });
      await $('button[aria-label="Recheck local status"]').click();
      await $("h1=Manage capture").waitForDisplayed();
      await browser.waitUntil(
        async () =>
          !(
            (await browser.execute(() => document.body.textContent)) ?? ""
          ).includes("Desktop settings could not be decoded"),
      );
      assert.equal(readFileSync(statePath, "utf8"), repaired);
      assert.equal(await $("h1=Set up Cutokyo").isExisting(), false);
      await $("button=Return to Settings").click();
      await $("h1=Settings").waitForDisplayed();
      await browser.waitUntil(
        async () =>
          (await $("button=Dark").getAttribute("aria-pressed")) === "true",
      );
      assert.equal(
        await browser.execute(() => document.documentElement.dataset.theme),
        "dark",
      );
      assert.deepEqual(JSON.parse(readFileSync(statePath, "utf8")), saved);
      assert.equal(existsSync(join(root, "data/capture-setup")), false);
      await browser.saveScreenshot(
        join(nativeEvidence, "state-warning-after-repair-1280x800.png"),
      );
    });
  } else if (process.env.CUTOKYO_NATIVE_PHASE === "capture-setup") {
    it("previews, cancels, installs, verifies, recovers and removes each real isolated native integration", async () => {
      const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
      assert.ok(root);
      await $("h1=Set up Cutokyo").waitForDisplayed();
      // Selecting an agent without the storage acknowledgement cannot install.
      await $('input[aria-label="Capture Claude Code"]').click();
      assert.equal(await $("button=Install selected").isEnabled(), false);
      await $('input[aria-label="Capture Claude Code"]').click();
      await $("//label[contains(., 'stored unencrypted')]/input").click();
      const original = '{\n  "user-only"  : "KEEP PACKAGE BYTE LAYOUT"\n}\n';
      const harnesses = [
        {
          name: "Claude Code",
          target: "home/.claude/settings.json",
          owned: "home/.claude/settings.json",
        },
        {
          name: "Codex",
          target: "home/.codex/hooks.json",
          owned: "home/.codex/hooks.json",
        },
        {
          name: "OpenCode",
          target: "config/opencode/opencode.json",
          owned: "config/opencode/plugins/cutokyo.ts",
        },
      ];
      for (const harness of harnesses) {
        const native = join(root, harness.target);
        await chooseAction(harness.name, "Install");
        await $('[role="dialog"]').waitForDisplayed();
        assert.equal(
          readFileSync(native, "utf8"),
          original,
          "native preview must not write config",
        );
        assert.equal(
          existsSync(join(root, "data/capture-setup")),
          harness.name !== "Claude Code",
          "the first preview creates no recovery state",
        );
        await (await dialogButton("Cancel")).click();
        await waitForDialogGone();
        assert.equal(readFileSync(native, "utf8"), original);
        await expectReady(harness.name, false);
        await chooseAction(harness.name, "Install");
        await (await dialogButton("Confirm")).click();
        await waitForDialogGone();
        assert.equal(await $('[role="alert"]').isExisting(), false);
        assert.ok(existsSync(join(root, harness.owned)));
        assert.ok(
          readFileSync(native, "utf8").includes(
            '"user-only"  : "KEEP PACKAGE BYTE LAYOUT"',
          ),
        );
        await expectReady(harness.name, true);
        await chooseAction(harness.name, "Repair");
        await (await dialogButton("Confirm")).click();
        await waitForDialogGone();
        assert.equal(await $('[role="alert"]').isExisting(), false);
        await chooseAction(harness.name, "Remove");
        await (await dialogButton("Confirm")).click();
        await waitForDialogGone();
        assert.equal(await $('[role="alert"]').isExisting(), false);
        assert.equal(
          readFileSync(native, "utf8"),
          original,
          "native removal restores unmanaged bytes",
        );
        if (harness.name === "OpenCode")
          assert.equal(existsSync(join(root, harness.owned)), false);
        await expectReady(harness.name, false);
        await chooseAction(harness.name, "Install");
        await (await dialogButton("Confirm")).click();
        await waitForDialogGone();
        await expectReady(harness.name, true);
        await $(`input[aria-label="Capture ${harness.name}"]`).click();
      }
      await browser.saveScreenshot(
        join(nativeEvidence, "onboarding-native-verified-1280x800.png"),
      );
      assert.equal(await $("button=Install selected").isExisting(), false);
      await $("button=Continue").click();
      await $("h1=Overview").waitForDisplayed();
      const state = JSON.parse(
        readFileSync(join(root, "data/desktop-state.json"), "utf8"),
      );
      assert.deepEqual(state.selected_harnesses, [
        "claude_code",
        "codex",
        "opencode",
      ]);
      assert.equal(state.onboarding_complete, true);
    });

    it("manages completed-user capture through Settings without resetting saved intent or unmanaged native bytes", async () => {
      const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
      assert.ok(root);
      const statePath = join(root, "data/desktop-state.json");
      const configPath = join(root, "config/config.toml");
      const savedState = readFileSync(statePath, "utf8");
      // Defaults need no app config file. Save a genuine non-default shared
      // preference through Settings before asserting its byte preservation.
      await $("a=Settings").click();
      await $("h1=Settings").waitForDisplayed();
      const searchMcp = await $('input[aria-label="Agent search"]');
      await searchMcp.waitForDisplayed();
      assert.equal(await searchMcp.isSelected(), true);
      await searchMcp.click();
      const preferenceReceipt = await $('main .success-message[role="status"]');
      await preferenceReceipt.waitForDisplayed();
      assert.equal(
        await preferenceReceipt.getText(),
        "Search MCP updated. Omitted settings were preserved.",
      );
      await browser.waitUntil(async () => !(await searchMcp.isSelected()));
      assert.equal(await searchMcp.isSelected(), false);
      const savedConfig = readFileSync(configPath, "utf8");
      assert.match(savedConfig, /^search_mcp_enabled = false$/m);
      const state = JSON.parse(savedState);
      assert.equal(state.onboarding_complete, true);
      assert.deepEqual(state.selected_harnesses, [
        "claude_code",
        "codex",
        "opencode",
      ]);
      const nativePath = join(root, "home/.claude/settings.json");
      const installedNative = readFileSync(nativePath, "utf8");
      const original = '{\n  "user-only"  : "KEEP PACKAGE BYTE LAYOUT"\n}\n';
      await $('a[aria-label="Manage capture"]').click();
      await $("h1=Manage capture").waitForDisplayed();
      assert.equal(await $("button=Continue").isExisting(), false);
      assert.equal(await $("button=Install selected").isExisting(), false);
      assert.equal(await $("button=Skip for now").isExisting(), false);
      assert.equal(readFileSync(statePath, "utf8"), savedState);
      await chooseAction("Claude Code", "Remove");
      await $('[role="dialog"]').waitForDisplayed();
      assert.equal(readFileSync(nativePath, "utf8"), installedNative);
      await (await dialogButton("Cancel")).click();
      await waitForDialogGone();
      assert.equal(readFileSync(nativePath, "utf8"), installedNative);
      await chooseAction("Claude Code", "Remove");
      await (await dialogButton("Confirm")).click();
      await waitForDialogGone();
      assert.equal(await $('[role="alert"]').isExisting(), false);
      assert.equal(readFileSync(nativePath, "utf8"), original);
      assert.equal(readFileSync(statePath, "utf8"), savedState);
      assert.equal(readFileSync(configPath, "utf8"), savedConfig);
      await expectReady("Claude Code", false);
      // Verify the missing integration through a fresh native preview, then
      // reinstall so the following actual installed-hook history phase is intact.
      await chooseAction("Claude Code", "Install");
      await $('[role="dialog"]').waitForDisplayed();
      await expectReady("Claude Code", false);
      assert.equal(readFileSync(nativePath, "utf8"), original);
      await browser.saveScreenshot(
        join(nativeEvidence, "completed-capture-removal-1280x800.png"),
      );
      await (await dialogButton("Confirm")).click();
      await waitForDialogGone();
      assert.equal(await $('[role="alert"]').isExisting(), false);
      await expectReady("Claude Code", true);
      assert.ok(
        readFileSync(nativePath, "utf8").includes(
          '"user-only"  : "KEEP PACKAGE BYTE LAYOUT"',
        ),
      );
      assert.equal(readFileSync(statePath, "utf8"), savedState);
      assert.equal(readFileSync(configPath, "utf8"), savedConfig);
      await $("button=Return to Settings").click();
      await $("h1=Settings").waitForDisplayed();
      assert.equal(new URL(await browser.getUrl()).hash, "#/settings");
    });
  } else if (process.env.CUTOKYO_NATIVE_PHASE === "capture-history") {
    it("opens real native-hook and transcript evidence and resumes its exact private native context", async () => {
      const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
      assert.ok(root);
      const nativeId = "12345678-1234-1234-1234-123456789abc";
      const project = join(root, "project with spaces ' and $literal");
      await $("h1=Overview").waitForDisplayed();
      await $("a=Sessions").click();
      await $("h1=Sessions").waitForDisplayed();
      const query = await $('input[aria-label="Search session content"]');
      await query.setValue("Packaged hook search needle 20261004");
      const result = await $('a[aria-label^="Open "]');
      await result.waitForDisplayed();
      await result.click();
      await $("h2=Session").waitForDisplayed();
      const detail = await $("main").getText();
      assert.ok(detail.includes("Packaged hook search needle 20261004"));
      assert.ok(detail.includes("Packaged native transcript needle 20261004"));
      assert.equal(
        await $(`main code[title="${nativeId}"]`).getAttribute("title"),
        nativeId,
      );
      await $("button=Resume").click();
      const dialog = await $('[role="dialog"]');
      await dialog.waitForDisplayed();
      const folder = await dialog.$("p*=Folder:");
      await folder.waitForDisplayed();
      assert.equal(await folder.getText(), `Folder: ${project}`);
      const confirmation = await $('[role="dialog"]').$("button=Resume");
      await confirmation.waitForEnabled();
      assert.equal(await confirmation.isEnabled(), true);
      await confirmation.click();
      const receipt = await $('main .success-message[role="status"]');
      await receipt.waitForDisplayed();
      assert.match(await receipt.getText(), /started in a visible terminal/);
      assert.match(
        await receipt.getText(),
        /cannot verify native session activation/,
      );
      const audit = process.env.CUTOKYO_NATIVE_RESUME_AUDIT;
      assert.ok(audit);
      await browser.waitUntil(
        () =>
          existsSync(audit) &&
          existsSync(audit.replace(/\.txt$/, ".context.json")),
        { timeout: 10_000 },
      );
      assert.equal(readFileSync(audit, "utf8"), `--resume\n${nativeId}\n`);
      const context = JSON.parse(
        readFileSync(audit.replace(/\.txt$/, ".context.json"), "utf8"),
      );
      assert.deepEqual(context, {
        cwd: project,
        stdinTty: true,
        stdoutTty: true,
      });
      await closeCompletedResumeTerminal(project, nativeId);
      await browser.saveScreenshot(
        join(nativeEvidence, "native-hook-detail-resume-1280x800.png"),
      );
    });
  } else {
    throw new Error(
      "The native package runner must select a maintained native journey phase.",
    );
  }
});
