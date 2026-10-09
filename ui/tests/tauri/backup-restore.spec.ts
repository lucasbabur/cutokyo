import { strict as assert } from "node:assert";
import {
  appendFileSync,
  existsSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";

import { $, $$, browser } from "@wdio/globals";

const root = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
const evidence = process.env.CUTOKYO_NATIVE_EVIDENCE;
assert.ok(root, "native backup tests require the isolated package-runner root");
assert.ok(evidence, "native backup evidence directory was not propagated");
const receiptFile = join(evidence, "backup-restore-receipt.json");

async function dataControls() {
  await $('.primary-nav a[href="#/data"]').click();
  await $("h1=Data controls").waitForDisplayed();
}
async function restoreFrom(location: string) {
  const rows = await $$(".deletion-session-list article");
  for (const row of rows) {
    if ((await row.getText()).includes(location)) {
      await row.$("button=Restore…").click();
      return;
    }
  }
  assert.fail(`Original backup row was not found: ${location}`);
}
async function done() {
  await $('[role="dialog"]').$("button=Done").click();
  await $('[role="dialog"]').waitForExist({ reverse: true });
}

describe("Native database backup and genuine restore", () => {
  if (process.env.CUTOKYO_NATIVE_PHASE === "save") {
    it("creates, cancels safely, restores actual history, and retains verified prior history", async () => {
      await dataControls();
      await $("button=Create backup").click();
      const create = await $('[role="dialog"]');
      await create.waitForDisplayed();
      await create.$("button=Create backup").click();
      await $("h2=Backup created").waitForDisplayed();
      const location = await create
        .$(".//dt[normalize-space(.)='Location']/following-sibling::dd[1]")
        .getText();
      assert.ok(location.startsWith(join(root, "data/backups/")));
      const database = join(location, "history.db");
      assert.ok(existsSync(database));
      assert.ok(existsSync(join(location, "history.db.manifest.json")));
      const manifest = JSON.parse(
        readFileSync(join(location, "history.db.manifest.json"), "utf8"),
      );
      assert.equal(manifest.byte_length, statSync(database).size);
      assert.equal(statSync(location).mode & 0o777, 0o700);
      assert.equal(statSync(database).mode & 0o777, 0o600);
      await browser.saveScreenshot(
        join(evidence, "backup-created-1280x800.png"),
      );
      await done();
      await restoreFrom(location);
      await $("button=Restore selected backup").waitForEnabled();
      await $("button=Cancel — keep current history").click();
      await $('[role="dialog"]').waitForExist({ reverse: true });
      const before = (await browser.tauri.execute(({ core }) =>
        core.invoke("search_sessions", {
          filters: {
            text: "JEV exact resume needle 73A9",
            queryMode: "terms",
            sort: "relevance",
            offset: 0,
            limit: 50,
            todayStart: null,
            harness: "all",
            project: "",
            branch: "",
            dateRange: "all",
            tool: "",
            skill: "",
            agent: "",
          },
        }),
      )) as { total: number };
      assert.equal(before.total, 1);
      await $("button=Delete all…").click();
      const deletion = await $('[role="dialog"]');
      await deletion.$("input").setValue("DELETE ALL LOCAL HISTORY");
      await deletion.$("button=Delete all local history").click();
      await $("p*=0 sessions stored").waitForDisplayed();
      const empty = (await browser.tauri.execute(({ core }) =>
        core.invoke("search_sessions", {
          filters: {
            text: "",
            queryMode: "terms",
            sort: "relevance",
            offset: 0,
            limit: 50,
            todayStart: null,
            harness: "all",
            project: "",
            branch: "",
            dateRange: "all",
            tool: "",
            skill: "",
            agent: "",
          },
        }),
      )) as { total: number };
      assert.equal(empty.total, 0);
      await restoreFrom(location);
      await $("button=Restore selected backup").waitForEnabled();
      await browser.saveScreenshot(
        join(evidence, "backup-restore-confirmation-1280x800.png"),
      );
      await $("button=Restore selected backup").click();
      await $("h2=History restored").waitForDisplayed();
      const recoveryPath = await $('[role="dialog"]')
        .$(".//dt[normalize-space(.)='Recovery copy']/following-sibling::dd[1]")
        .getText();
      assert.ok(existsSync(recoveryPath));
      assert.ok(existsSync(join(recoveryPath, "history.db")));
      assert.ok(existsSync(join(recoveryPath, "history.db.manifest.json")));
      assert.equal(
        await $('[role="dialog"]')
          .$(
            ".//dt[normalize-space(.)='Integrity result']/following-sibling::dd[1]",
          )
          .getText(),
        "ok",
      );
      await browser.saveScreenshot(
        join(evidence, "backup-restored-1280x800.png"),
      );
      writeFileSync(
        receiptFile,
        JSON.stringify({ location, recoveryPath, manifest }),
        { flag: "wx" },
      );
      await done();
      await $("p*=1 session stored").waitForDisplayed();
      await $("a=Sessions").click();
      await $('input[aria-label="Search session content"]').setValue(
        "JEV exact resume needle 73A9",
      );
      await $(
        'a[aria-label="Open Native exact resume 73A9"]',
      ).waitForDisplayed();
      await $('a[aria-label="Open Native exact resume 73A9"]').click();
      await $("h1=Native exact resume 73A9").waitForDisplayed();
    });

    it("reports tampering inside the restore modal without replacing native history", async () => {
      const { location } = JSON.parse(readFileSync(receiptFile, "utf8")) as {
        location: string;
      };
      const database = join(location, "history.db");
      const original = readFileSync(database);
      appendFileSync(database, "tampered native test bytes");
      try {
        await dataControls();
        await restoreFrom(location);
        await $('[role="dialog"] [role="alert"]').waitForDisplayed();
        assert.match(
          await $('[role="dialog"] [role="alert"]').getText(),
          /digest verification failed/,
        );
        assert.equal(
          await $("button=Restore selected backup").isEnabled(),
          false,
        );
        const unchanged = (await browser.tauri.execute(({ core }) =>
          core.invoke("search_sessions", {
            filters: {
              text: "JEV exact resume needle 73A9",
              queryMode: "terms",
              sort: "relevance",
              offset: 0,
              limit: 50,
              todayStart: null,
              harness: "all",
              project: "",
              branch: "",
              dateRange: "all",
              tool: "",
              skill: "",
              agent: "",
            },
          }),
        )) as { total: number };
        assert.equal(unchanged.total, 1);
        await $("button=Cancel — keep current history").click();
      } finally {
        writeFileSync(database, original);
      }
    });
  } else if (process.env.CUTOKYO_NATIVE_PHASE === "reopen") {
    it("rediscovers the private backup after a real application restart", async () => {
      const { location, recoveryPath } = JSON.parse(
        readFileSync(receiptFile, "utf8"),
      ) as { location: string; recoveryPath: string };
      assert.ok(existsSync(join(location, "history.db")));
      assert.ok(existsSync(recoveryPath));
      await dataControls();
      await $("button=Restore…").waitForDisplayed();
      assert.match(
        await $("main").getText(),
        new RegExp(location.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")),
      );
    });
  } else {
    throw new Error(
      "The isolated native runner must choose save or reopen phase",
    );
  }
});
