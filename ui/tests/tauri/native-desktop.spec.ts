import { strict as assert } from "node:assert";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { $, browser } from "@wdio/globals";

import { assertNativeGeometry } from "./geometry.js";

const nativeEvidence = process.env.CUTOKYO_NATIVE_EVIDENCE;
assert.ok(nativeEvidence, "native evidence directory was not propagated");
const evidencePath = join(nativeEvidence, "search-detail-resume-1280x800.png");

describe("Cutokyo native Tauri application", () => {
  it("searches, opens, exactly resumes, and deletes an isolated native fixture", async () => {
    const applicationUrl = new URL(await browser.getUrl());
    assert.equal(applicationUrl.protocol, "tauri:");
    assert.equal(applicationUrl.hostname, "localhost");

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
    const rawObservationCount = await deletionDialog.$(
      ".//dt[normalize-space(.)='Raw observations']/following-sibling::dd[1]",
    );
    const searchRowCount = await deletionDialog.$(
      ".//dt[normalize-space(.)='Search rows']/following-sibling::dd[1]",
    );
    assert.equal(await rawObservationCount.getText(), "1");
    assert.equal(await searchRowCount.getText(), "1");
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
});
