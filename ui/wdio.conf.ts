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

// Keep config importable by Knip without creating files or allocating ports.
// A missing runner never falls back to an auto-discovered source executable.
const application =
  process.env.CUTOKYO_NATIVE_APPLICATION ??
  "/cutokyo-native-package-runner-required";
const nativeEvidence = process.env.CUTOKYO_NATIVE_EVIDENCE;
const port = Number(process.env.CUTOKYO_NATIVE_PORT);

const serviceOptions: TauriServiceOptions = {
  appBinaryPath: application,
  driverProvider: "embedded",
  embeddedPort: port,
  startTimeout: 90_000,
  statusPollTimeout: 5_000,
  captureBackendLogs: true,
  ...(nativeEvidence ? { logDir: nativeEvidence } : {}),
};

const capability: TauriCapabilities = {
  browserName: "tauri",
  "tauri:options": { application },
};

const desktopSpec = "./tests/tauri/native-desktop.spec.ts";
const persistenceSpecs = [
  "./tests/tauri/backup-restore.spec.ts",
  "./tests/tauri/inventory-management.spec.ts",
  desktopSpec,
];

export const config: WebdriverIO.Config = {
  runner: "local",
  // Defer selection until WDIO reads the config, keeping static-tool imports
  // side-effect-free while refusing invalid phases before services can launch.
  get specs() {
    switch (process.env.CUTOKYO_NATIVE_PHASE) {
      case "save":
        return [
          ...persistenceSpecs,
          "./tests/tauri/zz-health-capabilities.spec.ts",
        ];
      case "reopen":
        return [...persistenceSpecs];
      case "browse":
      case "state-recovery":
      case "capture-setup":
      case "capture-history":
        return [desktopSpec];
      default:
        throw new Error(
          "CUTOKYO_NATIVE_PHASE must select save, reopen, browse, state-recovery, capture-setup, or capture-history.",
        );
    }
  },
  maxInstances: 1,
  services: [["@wdio/tauri-service", serviceOptions]],
  capabilities: [capability],
  framework: "mocha",
  reporters: ["spec"],
  ...(nativeEvidence ? { outputDir: nativeEvidence } : {}),
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
    if (
      required("CUTOKYO_NATIVE_APPLICATION") !==
        join(nativeTestRoot, "package/usr/bin/cutokyo-desktop") ||
      realpathSync(application) !== application
    ) {
      throw new Error(
        "Native WDIO must launch the extracted Debian bundle, not a build executable.",
      );
    }
    required("CUTOKYO_NATIVE_PORT");
    if (!Number.isInteger(port) || port < 1024 || port > 65535) {
      throw new Error("Native WDIO requires its allocated unprivileged port.");
    }
    const evidence = required("CUTOKYO_NATIVE_EVIDENCE");
    if (!existsSync(join(evidence, "package-launch.json"))) {
      throw new Error(
        "The package runner must record bundle provenance before launch.",
      );
    }
  },
  // No root deletion in WDIO hooks: their order relative to service teardown is
  // not an ownership guarantee. The outer runner stops children, then cleans up.
};
