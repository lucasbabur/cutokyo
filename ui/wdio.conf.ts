import { existsSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";

import LocalRunner from "@wdio/local-runner";
import type {
  TauriCapabilities,
  TauriServiceOptions,
} from "@wdio/tauri-service";
import type {} from "@wdio/types";

function required(name: string): string {
  const value = process.env[name];
  if (!value)
    throw new Error(`Run the isolated native package runner: missing ${name}`);
  return value;
}

const nativeTestRoot = resolve(required("CUTOKYO_DESKTOP_TEST_ROOT"));
if (
  dirname(nativeTestRoot) !== realpathSync(tmpdir()) ||
  realpathSync(nativeTestRoot) !== nativeTestRoot ||
  !basename(nativeTestRoot).startsWith("cutokyo-native-test-wdio-") ||
  process.env.CUTOKYO_DESKTOP_TEST_MODE !== "1"
) {
  throw new Error(
    "Native WDIO requires a real isolated temporary test root and test mode.",
  );
}
const application = required("CUTOKYO_NATIVE_APPLICATION");
if (
  application !== join(nativeTestRoot, "package/usr/bin/cutokyo-desktop") ||
  realpathSync(application) !== application
) {
  throw new Error(
    "Native WDIO must launch the extracted Debian bundle, not a build executable.",
  );
}
const nativeEvidence = required("CUTOKYO_NATIVE_EVIDENCE");
const port = Number(required("CUTOKYO_NATIVE_PORT"));
if (!Number.isInteger(port) || port < 1024 || port > 65535) {
  throw new Error("Native WDIO requires its allocated unprivileged port.");
}

const serviceOptions: TauriServiceOptions = {
  appBinaryPath: application,
  driverProvider: "embedded",
  embeddedPort: port,
  startTimeout: 90_000,
  statusPollTimeout: 5_000,
  captureBackendLogs: true,
  logDir: nativeEvidence,
};

const capability: TauriCapabilities = {
  browserName: "tauri",
  "tauri:options": { application },
};

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./tests/tauri/**/*.spec.ts"],
  maxInstances: 1,
  services: [["@wdio/tauri-service", serviceOptions]],
  capabilities: [capability],
  framework: "mocha",
  reporters: ["spec"],
  outputDir: nativeEvidence,
  logLevel: "info",
  bail: 0,
  waitforTimeout: 15_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 1,
  mochaOpts: { ui: "bdd", timeout: 90_000 },
  onPrepare() {
    if (typeof LocalRunner !== "function") {
      throw new TypeError("WebdriverIO local runner is unavailable.");
    }
    if (!existsSync(join(nativeEvidence, "package-launch.json"))) {
      throw new Error(
        "The package runner must record bundle provenance before launch.",
      );
    }
  },
  // No root deletion in WDIO hooks: their order relative to service teardown is
  // not an ownership guarantee. The outer runner stops children, then cleans up.
};
