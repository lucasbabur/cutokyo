import { strict as assert } from "node:assert";
import { existsSync, writeFileSync } from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
const { $, browser } = createRequire(new URL("../../ui/package.json", import.meta.url))("@wdio/globals");
import { assertNativeGeometry } from "../../ui/tests/tauri/geometry.js";

const failure = "The terminal launcher exited unsuccessfully before acknowledging an interactive harness. Check its display access and configuration.";
const evidence = process.env.CUTOKYO_NATIVE_EVIDENCE;
assert.ok(evidence);

async function preview() {
  await $("button=Resume").click();
  const dialog = await $('[role="dialog"]');
  await dialog.waitForDisplayed();
  await (await dialog.$('.//code[normalize-space(.)="claude-native-73A9"]')).waitForDisplayed();
  assert.match(await dialog.getText(), /recorded --resume target/);
  assert.match(await dialog.getText(), /not session activation/);
  return dialog;
}

async function rejectLaunch(dialog) {
  const confirm = await dialog.$("button=Resume in Claude Code");
  await confirm.waitForEnabled();
  await confirm.click();
  const alert = await dialog.$('[role="alert"]');
  await alert.waitForDisplayed();
  assert.equal(await alert.getText(), failure);
  assert.match(await dialog.getText(), /claude-native-73A9/);
  assert.equal(await $('main .success-message[role="status"]').isExisting(), false);
  await confirm.waitForEnabled();
  assert.equal(await dialog.$("button=Cancel").isEnabled(), true);
  assert.equal(await $$Alerts(), 1);
  const audit = process.env.CUTOKYO_NATIVE_RESUME_AUDIT;
  assert.ok(audit);
  assert.equal(existsSync(audit), false);
  assert.equal(existsSync(audit.replace(/\.txt$/, ".context.json")), false);
}

async function $$Alerts() {
  return browser.execute(() => document.querySelectorAll('[role="alert"]').length);
}

async function assertRestoredFocus() {
  assert.equal(await browser.execute(() => document.activeElement?.textContent?.trim()), "Resume");
}

describe("Packaged resume failure repair", () => {
  it("keeps a genuine launcher failure inside confirmation, supports retry and restores focus on cancellation", async () => {
    const url = new URL(await browser.getUrl());
    assert.equal(url.protocol, "tauri:");
    assert.equal(url.hostname, "localhost");
    await browser.execute(() => { location.hash = "/sessions"; });
    await $("h1=Sessions").waitForDisplayed();
    await $('input[aria-label="Search session content"]').setValue("JEV exact resume needle 73A9");
    const result = await $('a[aria-label="Open Native exact resume 73A9"]');
    await result.waitForDisplayed();
    await result.click();
    await $("h1=Native exact resume 73A9").waitForDisplayed();
    const detailUrl = await browser.getUrl();
    let dialog = await preview();
    await rejectLaunch(dialog);
    const windowSize = await browser.tauri.execute(({ core }) => core.invoke("plugin:window|inner_size", { label: "main" }));
    const viewport = await browser.execute(() => ({ width: window.innerWidth, height: window.innerHeight }));
    assertNativeGeometry(windowSize, viewport);
    writeFileSync(path.join(evidence, "negative-geometry-1280x800.json"), JSON.stringify({ windowSize, viewport }), { flag: "wx", mode: 0o600 });
    await browser.saveScreenshot(path.join(evidence, "resume-inline-launch-failure-1280x800.png"));
    writeFileSync(path.join(evidence, "resume-inline-failure-dom.html"), await browser.getPageSource(), { flag: "wx", mode: 0o600 });
    await rejectLaunch(dialog);
    assert.equal(await browser.getUrl(), detailUrl);
    await dialog.$("button=Cancel").click();
    await dialog.waitForExist({ reverse: true });
    await assertRestoredFocus();
    dialog = await preview();
    assert.equal(await dialog.$('[role="alert"]').isExisting(), false);
    await dialog.$('[aria-label="Close dialog"]').click();
    await dialog.waitForExist({ reverse: true });
    await assertRestoredFocus();
    await $("a=Back to sessions").click();
    await $("h1=Sessions").waitForDisplayed();
    assert.equal(await $('input[aria-label="Search session content"]').getValue(), "JEV exact resume needle 73A9");
    writeFileSync(path.join(evidence, "resume-negative-retest-assertions.json"), JSON.stringify({
      genuineLauncherFailure: failure, inlineDialogAlert: true, alertCount: 1,
      noSuccessReceipt: true, noHarnessAudit: true, retryFailedTruthfully: true,
      cancelAndCloseRestoreResumeFocus: true, newPreviewClearsError: true,
      exactNativeId: "claude-native-73A9", searchQueryRetained: true,
      separateNegativeJourneyNotFreshAggregate: true,
    }, null, 2), { flag: "wx", mode: 0o600 });
  });
});
