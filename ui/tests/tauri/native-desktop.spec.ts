import { strict as assert } from "node:assert";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { $, browser } from "@wdio/globals";

const repositoryRoot = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../../..",
);
const evidencePath = resolve(
  repositoryRoot,
  "evidence/final/native/search-detail-resume-1280x800.png",
);

describe("Cutokyo native Tauri application", () => {
  it("searches, opens, exactly resumes, and deletes an isolated native fixture", async () => {
    const overview = await $("h1=Overview");
    await overview.waitForDisplayed();
    assert.equal(await overview.getText(), "Overview");

    const windowSize = await browser.getWindowSize();
    assert.ok(windowSize.width >= 900, `native width was ${windowSize.width}`);
    assert.ok(
      windowSize.height >= 700,
      `native height was ${windowSize.height}`,
    );

    const sessionsNavigation = await $("a=Sessions");
    await sessionsNavigation.click();
    const sessionsHeading = await $("h1=Sessions");
    await sessionsHeading.waitForDisplayed();

    const search = await $('input[aria-label="Search session content"]');
    await search.setValue("JEV exact resume needle 73A9");
    const resultLink = await $('a[aria-label="Open Native exact resume 73A9"]');
    await resultLink.waitForDisplayed();
    await resultLink.click();

    const detailHeading = await $("h1=Native exact resume 73A9");
    await detailHeading.waitForDisplayed();
    assert.match(await $("main").getText(), /claude-native-73A9/);

    await $("button=Resume").click();
    const resumeDialog = await $('[role="dialog"]');
    await resumeDialog.waitForDisplayed();
    const previewText = await resumeDialog.getText();
    assert.match(previewText, /claude-native-73A9/);
    assert.match(previewText, /recorded --resume target/);
    await browser.saveScreenshot(evidencePath);

    await $("button=Resume in Claude Code").click();
    const success = await $('[role="status"]');
    await success.waitForDisplayed();
    assert.match(
      await success.getText(),
      /exact native session claude-native-73A9/,
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

    await $("button=Delete session").click();
    const deletionDialog = await $('[role="dialog"]');
    await deletionDialog.waitForDisplayed();
    assert.match(await deletionDialog.getText(), /Raw observations\s+1/);
    assert.match(await deletionDialog.getText(), /Search rows\s+1/);
    await $("button=Delete selected session").click();

    await sessionsHeading.waitForDisplayed();
    const emptyState = await $("h2=No sessions match these filters");
    await emptyState.waitForDisplayed();
  });
});
