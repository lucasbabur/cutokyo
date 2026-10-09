import { strict as assert } from "node:assert";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { $, browser } from "@wdio/globals";

import { assertNativeGeometry } from "./geometry.js";

const nativeEvidence = process.env.CUTOKYO_NATIVE_EVIDENCE;
assert.ok(nativeEvidence, "native evidence directory was not propagated");
const evidencePath = join(nativeEvidence, "search-detail-resume-1280x800.png");

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
      assert.match(await $("main").getText(), /claude-native-73A9/);

      await $("button=Resume").click();
      const resumeDialog = await $('[role="dialog"]');
      await resumeDialog.waitForDisplayed();
      const previewId = await resumeDialog.$(
        './/code[normalize-space(.)="claude-native-73A9"]',
      );
      await previewId.waitForDisplayed();
      const previewText = await resumeDialog.getText();
      assert.match(previewText, /claude-native-73A9/);
      assert.match(previewText, /recorded --resume target/);
      await browser.saveScreenshot(evidencePath);

      const confirmation = await $("button=Resume in Claude Code");
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

      await $("button=Delete session").click();
      const deletionDialog = await $('[role="dialog"]');
      await deletionDialog.waitForDisplayed();
      const rawObservationCount = await deletionDialog.$(
        ".//dt[normalize-space(.)='Raw observations']/following-sibling::dd[1]",
      );
      const searchRowCount = await deletionDialog.$(
        ".//dt[normalize-space(.)='Search rows']/following-sibling::dd[1]",
      );
      assert.equal(await rawObservationCount.getText(), "1");
      assert.equal(await searchRowCount.getText(), "3");
      await $("button=Delete selected session").click();

      const returnedSessionsHeading = await $("h1=Sessions");
      await returnedSessionsHeading.waitForDisplayed();
      const deletedResult = await $(
        'a[aria-label="Open Native exact resume 73A9"]',
      );
      await deletedResult.waitForExist({ reverse: true });
      const emptyState = await $("h2=History is empty—not zero");
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
      await $(
        "h1=See your agent work without sending it away",
      ).waitForDisplayed();
      const browse = await $("button=Browse without installing");
      assert.equal(await browse.isEnabled(), false);
      await $("//label[contains(., 'I understand')]/input").click();
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
      await $("h2=History is empty—not zero").waitForDisplayed();
      await browser.saveScreenshot(
        join(nativeEvidence, "onboarding-browse-1280x800.png"),
      );
    });
  } else if (process.env.CUTOKYO_NATIVE_PHASE === "capture-setup") {
    it("previews, cancels, installs, verifies, recovers and removes each real isolated native integration", async () => {
      const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
      assert.ok(root);
      await $(
        "h1=See your agent work without sending it away",
      ).waitForDisplayed();
      await $("//label[contains(., 'I understand')]/input").click();
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
        await $(`button=Preview ${harness.name} install`).click();
        await $('[aria-label="Capture setup preview"]').waitForDisplayed();
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
        await $("button=Cancel preview").click();
        assert.equal(readFileSync(native, "utf8"), original);
        await $(`button=Preview ${harness.name} install`).click();
        await $(
          `button=Confirm install for ${harness.name}`,
        ).waitForDisplayed();
        await $(`button=Confirm install for ${harness.name}`).click();
        await $('[aria-label="Capture setup preview"]').waitForExist({
          reverse: true,
        });
        assert.equal(await $('[role="alert"]').isExisting(), false);
        assert.ok(existsSync(join(root, harness.owned)));
        assert.ok(
          readFileSync(native, "utf8").includes(
            '"user-only"  : "KEEP PACKAGE BYTE LAYOUT"',
          ),
        );
        await $(`button=Recover ${harness.name}`).click();
        await $(
          `button=Confirm recover for ${harness.name}`,
        ).waitForDisplayed();
        await $(`button=Confirm recover for ${harness.name}`).click();
        await $('[aria-label="Capture setup preview"]').waitForExist({
          reverse: true,
        });
        assert.equal(await $('[role="alert"]').isExisting(), false);
        await $(`button=Preview ${harness.name} removal`).click();
        await $(
          `button=Confirm uninstall for ${harness.name}`,
        ).waitForDisplayed();
        await $(`button=Confirm uninstall for ${harness.name}`).click();
        await $('[aria-label="Capture setup preview"]').waitForExist({
          reverse: true,
        });
        assert.equal(await $('[role="alert"]').isExisting(), false);
        assert.equal(
          readFileSync(native, "utf8"),
          original,
          "native removal restores unmanaged bytes",
        );
        if (harness.name === "OpenCode")
          assert.equal(existsSync(join(root, harness.owned)), false);
        await $(`button=Preview ${harness.name} install`).click();
        await $(
          `button=Confirm install for ${harness.name}`,
        ).waitForDisplayed();
        await $(`button=Confirm install for ${harness.name}`).click();
        await $('[aria-label="Capture setup preview"]').waitForExist({
          reverse: true,
        });
        await $(
          `//label[contains(@class, 'harness-choice') and contains(., '${harness.name}')]/input`,
        ).click();
      }
      assert.match(
        await $("main").getText(),
        /Configuration verified\. Live capture unknown\./,
      );
      await browser.saveScreenshot(
        join(nativeEvidence, "onboarding-native-verified-1280x800.png"),
      );
      await $("button=Finish with verified capture").click();
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
      const savedConfig = readFileSync(configPath, "utf8");
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
      await $("a=Settings").click();
      await $("h1=Settings").waitForDisplayed();
      await $("a=Manage native capture").click();
      await $("h1=Manage native capture").waitForDisplayed();
      assert.equal(
        await $("button=Finish with verified capture").isExisting(),
        false,
      );
      assert.equal(
        await $("button=Browse without installing").isExisting(),
        false,
      );
      assert.equal(readFileSync(statePath, "utf8"), savedState);
      await $("button=Preview Claude Code removal").click();
      await $('[aria-label="Capture setup preview"]').waitForDisplayed();
      assert.equal(
        await $("button=Confirm uninstall for Claude Code").isEnabled(),
        false,
      );
      assert.equal(readFileSync(nativePath, "utf8"), installedNative);
      await $("button=Cancel preview").click();
      assert.equal(readFileSync(nativePath, "utf8"), installedNative);
      await $("//label[contains(., 'I understand')]/input").click();
      await $("button=Preview Claude Code removal").click();
      await $("button=Confirm uninstall for Claude Code").waitForDisplayed();
      await $("button=Confirm uninstall for Claude Code").click();
      await $('[aria-label="Capture setup preview"]').waitForExist({
        reverse: true,
      });
      assert.equal(await $('[role="alert"]').isExisting(), false);
      assert.equal(readFileSync(nativePath, "utf8"), original);
      assert.equal(readFileSync(statePath, "utf8"), savedState);
      assert.equal(readFileSync(configPath, "utf8"), savedConfig);
      const capture = await $(
        '[aria-label="Claude Code capture configuration"]',
      );
      assert.match(
        await capture.getText(),
        /Configuration not verified\. Live capture unknown\./,
      );
      // Verify the missing integration through a fresh native preview, then
      // reinstall so the following actual installed-hook history phase is intact.
      await $("button=Preview Claude Code install").click();
      await $('[aria-label="Capture setup preview"]').waitForDisplayed();
      assert.match(
        await capture.getText(),
        /Configuration not verified\. Live capture unknown\./,
      );
      assert.equal(readFileSync(nativePath, "utf8"), original);
      await browser.saveScreenshot(
        join(nativeEvidence, "completed-capture-removal-1280x800.png"),
      );
      await $("button=Confirm install for Claude Code").click();
      await $('[aria-label="Capture setup preview"]').waitForExist({
        reverse: true,
      });
      assert.equal(await $('[role="alert"]').isExisting(), false);
      assert.match(
        await capture.getText(),
        /Configuration verified\. Live capture unknown\./,
      );
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
      await $("h2=Native target").waitForDisplayed();
      const detail = await $("main").getText();
      assert.ok(detail.includes("Packaged hook search needle 20261004"));
      assert.ok(detail.includes("Packaged native transcript needle 20261004"));
      assert.ok(detail.includes(nativeId));
      await $("button=Resume").click();
      const dialog = await $('[role="dialog"]');
      await dialog.waitForDisplayed();
      const previewId = await dialog.$(
        `.//code[normalize-space(.)="${nativeId}"]`,
      );
      await previewId.waitForDisplayed();
      const preview = await dialog.getText();
      assert.ok(preview.includes(nativeId));
      assert.ok(preview.includes(project));
      const confirmation = await $("button=Resume in Claude Code");
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
