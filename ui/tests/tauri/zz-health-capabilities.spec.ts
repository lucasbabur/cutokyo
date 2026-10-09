import { strict as assert } from "node:assert";
import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";

import { $, browser } from "@wdio/globals";

import { createTrustedKeyboard } from "./trusted-keyboard";

interface NativeKeyEvent {
  type: string;
  key: string | null;
  isTrusted: boolean;
  id: string;
  tag: string;
  className: string;
  time: number;
}

type RecorderWindow = Window & {
  __cutokyoTrustedKeyRecorder?: {
    events: NativeKeyEvent[];
    listener: (event: Event) => void;
  };
};

async function keyboardSnapshot() {
  return browser.execute(() => {
    const recorder = (window as RecorderWindow).__cutokyoTrustedKeyRecorder;
    return {
      id: document.activeElement?.id,
      tag: document.activeElement?.tagName,
      className: document.activeElement?.className,
      url: location.href,
      events: [...(recorder?.events ?? [])],
    };
  });
}

const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
const evidence = process.env.CUTOKYO_NATIVE_EVIDENCE;
assert.ok(root, "native health tests require the isolated package-runner root");
assert.ok(evidence, "native health evidence directory was not propagated");

describe("Native truthful capabilities and private exports", () => {
  if (process.env.CUTOKYO_NATIVE_PHASE === "save") {
    it("keeps trusted keyboard skip navigation on the current native route", async () => {
      const proof: Record<string, unknown> = {
        verdict: "FAIL",
        dispatches: [],
      };
      let armed = false;
      let failure: { error: unknown } | null = null;
      try {
        const keyboard = createTrustedKeyboard();
        proof.dispatches = keyboard.dispatches;
        // Route setup is not keyboard activation; neither skip nor main is focused here.
        await browser.execute(() => {
          location.hash = "#/sessions";
        });
        await $("h1=Sessions").waitForDisplayed();
        await browser.execute(
          () =>
            new Promise<void>((resolve) => {
              requestAnimationFrame(() =>
                requestAnimationFrame(() => resolve()),
              );
            }),
        );
        const url = await browser.getUrl();
        await browser.execute(() => {
          const host = window as RecorderWindow;
          if (host.__cutokyoTrustedKeyRecorder)
            throw new Error("stale native keyboard recorder");
          const events: NativeKeyEvent[] = [];
          const listener = (event: Event) => {
            if (!(event.target instanceof Element)) return;
            events.push({
              type: event.type,
              key: event instanceof KeyboardEvent ? event.key : null,
              isTrusted: event.isTrusted,
              id: event.target.id,
              tag: event.target.tagName,
              className: event.target.getAttribute("class") ?? "",
              time: performance.now(),
            });
          };
          host.__cutokyoTrustedKeyRecorder = { events, listener };
          for (const type of [
            "keydown",
            "keyup",
            "click",
            "focusin",
            "focusout",
          ])
            document.addEventListener(type, listener, true);
        });
        armed = true;
        await browser.execute(() => {
          const previous = document.body.getAttribute("tabindex");
          document.body.setAttribute("tabindex", "-1");
          document.body.focus();
          if (previous === null) document.body.removeAttribute("tabindex");
          else document.body.setAttribute("tabindex", previous);
        });
        const before = await keyboardSnapshot();
        proof.before = before;
        assert.equal(before.tag, "BODY");
        assert.equal(before.url, url);
        keyboard.send("Tab");
        await browser.waitUntil(
          async () => {
            const snapshot = await keyboardSnapshot();
            return snapshot.tag === "A" && snapshot.className === "skip-link";
          },
          {
            timeout: 3_000,
            timeoutMsg: "trusted Tab did not traverse body to skip link",
          },
        );
        const afterTab = await keyboardSnapshot();
        proof.afterTab = afterTab;
        assert.equal(afterTab.url, url);
        assert.ok(
          afterTab.events.some(
            (event) =>
              event.type === "keydown" &&
              event.key === "Tab" &&
              event.isTrusted,
          ),
        );
        keyboard.send("Return");
        await browser.waitUntil(
          async () => (await keyboardSnapshot()).id === "main-content",
          {
            timeout: 3_000,
            timeoutMsg: "trusted Return did not activate skip navigation",
          },
        );
        const afterReturn = await keyboardSnapshot();
        proof.afterReturn = afterReturn;
        const tab = afterReturn.events.findIndex(
          (event) =>
            event.type === "keydown" && event.key === "Tab" && event.isTrusted,
        );
        const enter = afterReturn.events.findIndex(
          (event) =>
            event.type === "keydown" &&
            event.key === "Enter" &&
            event.isTrusted &&
            event.className === "skip-link",
        );
        const activation = afterReturn.events.findIndex(
          (event) =>
            event.type === "click" &&
            event.isTrusted &&
            event.className === "skip-link",
        );
        assert.ok(
          tab >= 0 && enter > tab && activation > enter,
          "missing ordered trusted Tab/Enter/skip activation events",
        );
        assert.equal(afterReturn.tag, "MAIN");
        assert.equal(afterReturn.id, "main-content");
        assert.equal(afterReturn.url, url);
        assert.equal(await browser.getUrl(), url);
        await $("h1=Sessions").waitForDisplayed();
        proof.verdict = "PASS";
      } catch (error) {
        proof.error = error instanceof Error ? error.message : String(error);
        if (armed) {
          try {
            proof.failureSnapshot = await keyboardSnapshot();
          } catch (snapshotError) {
            proof.snapshotError = String(snapshotError);
          }
        }
        failure = { error };
      }
      try {
        if (armed)
          await browser.execute(() => {
            const host = window as RecorderWindow;
            const recorder = host.__cutokyoTrustedKeyRecorder;
            if (recorder)
              for (const type of [
                "keydown",
                "keyup",
                "click",
                "focusin",
                "focusout",
              ])
                document.removeEventListener(type, recorder.listener, true);
            delete host.__cutokyoTrustedKeyRecorder;
          });
      } catch (error) {
        proof.verdict = "FAIL";
        proof.cleanupError = String(error);
        failure ??= { error };
      }
      writeFileSync(
        join(evidence, "native-keyboard-trusted.json"),
        JSON.stringify(proof, null, 2) + "\n",
        {
          flag: "wx",
          mode: 0o600,
        },
      );
      if (failure) throw failure.error;
    });

    it("discloses unavailable signed updates before dead-end actions", async () => {
      const capabilities = (await browser.tauri.execute(({ core }) =>
        core.invoke("desktop_capabilities"),
      )) as {
        updates: { available: boolean; reason: string };
      };
      assert.equal(capabilities.updates.available, false);
      await $("a=Settings").click();
      await $("h1=Settings").waitForDisplayed();
      assert.ok(capabilities.updates.reason.length > 0);
      assert.ok(
        (await $("main").getText()).includes(
          "Install updates manually for now.",
        ),
      );
      assert.equal(
        await $('select[aria-label="Updater behavior"]').isEnabled(),
        false,
      );
      assert.equal(await $("button=Check now").isEnabled(), false);
      assert.equal(await $("button=Enable proxy").isExisting(), false);
      assert.equal(await $("a=Guards").isExisting(), false);
    });

    it("cancels safely then exports exactly the previewed private diagnostic report", async () => {
      await $("a=Health").click();
      await $("h1=Health").waitForDisplayed();
      const health = (await browser.tauri.execute(({ core }) =>
        core.invoke("health_snapshot"),
      )) as {
        dimensions: { id: string; name: string }[];
      };
      const quarantine = health.dimensions.find(
        (dimension) => dimension.id === "quarantine",
      );
      assert.ok(
        quarantine,
        "native health must identify the quarantine dimension",
      );
      assert.ok(
        (await $("main").getText())
          .toLowerCase()
          .includes(quarantine.name.toLowerCase()),
        "quarantine dimension must be listed on the Health page",
      );
      const directory = join(root, "data/diagnostics");
      const before = existsSync(directory) ? readdirSync(directory) : [];
      await $("button=Export report").click();
      await $('[role="dialog"]').waitForDisplayed();
      await browser.waitUntil(async () =>
        (await $('[role="dialog"]').getText()).includes(
          "cutokyo-diagnostic-bundle.json",
        ),
      );
      await $("button=Cancel").click();
      await $('[role="dialog"]').waitForExist({ reverse: true });
      assert.deepEqual(
        existsSync(directory) ? readdirSync(directory) : [],
        before,
      );
      await $("button=Export report").click();
      await $("button=Export").waitForEnabled();
      await browser.saveScreenshot(
        join(evidence, "diagnostic-preview-1280x800.png"),
      );
      await $("button=Export").click();
      await $('[role="dialog"]').waitForExist({ reverse: true });
      const receipt = await $(".success-message").getText();
      const location = receipt
        .split("Created local diagnostic bundle at ")[1]
        ?.trim();
      assert.ok(
        location,
        "successful export must report its actual destination",
      );
      assert.ok(location.startsWith(directory + "/"));
      assert.deepEqual(readdirSync(join(location, "..")), [
        "cutokyo-diagnostic-bundle.json",
      ]);
      assert.equal(statSync(location).mode & 0o777, 0o600);
      assert.equal(statSync(join(location, "..")).mode & 0o777, 0o700);
      const payload = readFileSync(location, "utf8");
      assert.ok(!payload.includes(root));
      assert.ok(!payload.includes("JEV exact resume needle 73A9"));
      assert.ok(!payload.includes("claude-native-73A9"));
      assert.ok(Array.isArray(JSON.parse(payload).diagnostics));
      await browser.saveScreenshot(
        join(evidence, "diagnostic-created-1280x800.png"),
      );
    });
  }
});
