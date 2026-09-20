import { chmodSync, copyFileSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import LocalRunner from "@wdio/local-runner";
import type {
  TauriCapabilities,
  TauriServiceOptions,
} from "@wdio/tauri-service";
import type {} from "@wdio/types";

const uiRoot = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(uiRoot, "..");
const executableSuffix = process.platform === "win32" ? ".exe" : "";
const application = join(
  repositoryRoot,
  "target",
  "debug",
  `cutokyo-desktop${executableSuffix}`,
);
const fakeHarnessSource = join(
  repositoryRoot,
  "target",
  "debug",
  `fake-resume-harness${executableSuffix}`,
);

const nativeTestParent = resolve(tmpdir());
const inheritedNativeTestRoot = process.env.CUTOKYO_DESKTOP_TEST_ROOT;
const nativeTestRoot = inheritedNativeTestRoot
  ? resolve(inheritedNativeTestRoot)
  : join(nativeTestParent, `cutokyo-native-test-wdio-${process.pid}`);
if (
  dirname(nativeTestRoot) !== nativeTestParent ||
  !basename(nativeTestRoot).startsWith("cutokyo-native-test-wdio-")
) {
  throw new Error(
    "The propagated native test root must be a direct, isolated child of the system temporary directory.",
  );
}
process.env.CUTOKYO_DESKTOP_TEST_ROOT = nativeTestRoot;
const nativeResumeAudit = join(nativeTestRoot, "resume-audit.txt");
process.env.CUTOKYO_NATIVE_RESUME_AUDIT = nativeResumeAudit;
const fakeBin = join(nativeTestRoot, "bin");
const fakeClaude = join(fakeBin, `claude${executableSuffix}`);
const nativeEvidence = join(repositoryRoot, "evidence", "final", "native");

function prepareNativeTest(): void {
  if (typeof LocalRunner !== "function") {
    throw new TypeError("WebdriverIO local runner is unavailable.");
  }
  rmSync(nativeTestRoot, { force: true, recursive: true });
  mkdirSync(fakeBin, { recursive: true, mode: 0o700 });
  mkdirSync(nativeEvidence, { recursive: true });
  copyFileSync(fakeHarnessSource, fakeClaude);
  if (process.platform !== "win32") chmodSync(fakeClaude, 0o700);
}

const serviceOptions: TauriServiceOptions = {
  appBinaryPath: application,
  driverProvider: "embedded",
  startTimeout: 90_000,
  statusPollTimeout: 5_000,
  env: {
    CUTOKYO_DESKTOP_TEST_MODE: "1",
    CUTOKYO_DESKTOP_TEST_ROOT: nativeTestRoot,
    CUTOKYO_DESKTOP_TEST_FIXTURE: "search-resume",
    CUTOKYO_NATIVE_RESUME_AUDIT: nativeResumeAudit,
    PATH: `${fakeBin}${delimiter}${process.env.PATH ?? ""}`,
  },
};

const capability: TauriCapabilities = {
  browserName: "tauri",
  "tauri:options": {
    application,
  },
};

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./tests/tauri/**/*.spec.ts"],
  maxInstances: 1,
  services: [["@wdio/tauri-service", serviceOptions]],
  capabilities: [capability],
  framework: "mocha",
  reporters: ["spec"],
  logLevel: "warn",
  bail: 0,
  waitforTimeout: 15_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 1,
  mochaOpts: {
    ui: "bdd",
    timeout: 90_000,
  },
  onPrepare() {
    prepareNativeTest();
  },
  onComplete() {
    if (process.env.CUTOKYO_KEEP_NATIVE_TEST_ROOT !== "1") {
      rmSync(nativeTestRoot, { force: true, recursive: true });
    }
  },
};
