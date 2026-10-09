import { strict as assert } from "node:assert";
import { execFileSync } from "node:child_process";
import {
  accessSync,
  constants,
  readdirSync,
  readlinkSync,
  realpathSync,
  statSync,
} from "node:fs";
import { isAbsolute } from "node:path";

export interface TrustedKeyboardIO {
  processes(): readonly string[];
  executable(pid: number): string;
  validateCli(path: string): void;
  hyprctl(cli: string, args: readonly string[], env: NodeJS.ProcessEnv): string;
}

const systemIO: TrustedKeyboardIO = {
  processes: () => readdirSync("/proc"),
  executable: (pid) => readlinkSync(`/proc/${pid}/exe`),
  validateCli: (path) => {
    assert.equal(
      realpathSync(path),
      path,
      "compositor CLI must be its resolved absolute path",
    );
    assert.ok(
      statSync(path).isFile(),
      "compositor CLI must be a regular executable file",
    );
    accessSync(path, constants.X_OK);
  },
  hyprctl: (cli, args, env) =>
    execFileSync(cli, [...args], {
      env,
      encoding: "utf8",
      timeout: 5_000,
      maxBuffer: 1024 * 1024,
    }),
};

interface WindowIdentity {
  address: string;
  pid: number;
  mapped: boolean;
  xwayland: boolean;
}

interface DispatchTrace {
  cli: string;
  key: "Tab" | "Return";
  pid: number;
  address: string;
  activeBefore: WindowIdentity;
  activeAfter: WindowIdentity | null;
  acknowledgement: string | null;
}

function record(value: unknown): Record<string, unknown> {
  assert.ok(
    value !== null && typeof value === "object" && !Array.isArray(value),
  );
  return value as Record<string, unknown>;
}

function windowIdentity(value: unknown): WindowIdentity {
  const window = record(value);
  assert.ok(
    typeof window.address === "string" &&
      window.address.match(/^0x[0-9a-fA-F]+/u)?.[0] === window.address,
    "invalid compositor window address",
  );
  assert.ok(
    typeof window.pid === "number" &&
      Number.isSafeInteger(window.pid) &&
      window.pid > 0,
    "invalid compositor window PID",
  );
  assert.equal(typeof window.mapped, "boolean", "missing mapped window state");
  assert.equal(
    typeof window.xwayland,
    "boolean",
    "missing XWayland window state",
  );
  return {
    address: window.address,
    pid: window.pid,
    mapped: window.mapped as boolean,
    xwayland: window.xwayland as boolean,
  };
}

/** Test-only compositor input: never focuses a window or sends a global key. */
export function createTrustedKeyboard(
  environment: NodeJS.ProcessEnv = process.env,
  io: TrustedKeyboardIO = systemIO,
) {
  const application = environment.CUTOKYO_NATIVE_APPLICATION;
  const runtime = environment.CUTOKYO_NATIVE_COMPOSITOR_RUNTIME;
  const configuredCli = environment.CUTOKYO_NATIVE_COMPOSITOR_CLI;
  if (
    !configuredCli ||
    !isAbsolute(configuredCli) ||
    !environment.HYPRLAND_INSTANCE_SIGNATURE ||
    !application ||
    !isAbsolute(application) ||
    !runtime ||
    !isAbsolute(runtime) ||
    runtime === environment.XDG_RUNTIME_DIR
  ) {
    throw new Error(
      "Trusted native keyboard blocked: isolated application, host Hyprland runtime and resolved compositor CLI are required",
    );
  }
  const cli = configuredCli;
  try {
    io.validateCli(cli);
  } catch (error) {
    throw new Error(
      "Trusted native keyboard blocked: compositor CLI is not a resolved regular executable",
      { cause: error },
    );
  }
  // Only the hyprctl child sees the host socket directory. WDIO/app HOME and XDG remain private.
  const compositorEnv = { ...environment, XDG_RUNTIME_DIR: runtime };
  const query = (name: "clients" | "activewindow"): unknown =>
    JSON.parse(io.hyprctl(cli, [name, "-j"], compositorEnv));
  let owner: { pid: number; address: string } | null = null;
  let blocked = false;
  const dispatches: DispatchTrace[] = [];

  function verifiedOwner() {
    const matches: number[] = [];
    for (const entry of io.processes()) {
      const pid = Number(entry);
      if (!Number.isSafeInteger(pid) || pid < 1 || String(pid) !== entry)
        continue;
      try {
        if (io.executable(pid) === application) matches.push(pid);
      } catch (error) {
        // Processes can exit while /proc is scanned; foreign users' exe links can be private.
        const code = (error as NodeJS.ErrnoException).code;
        if (!["ENOENT", "ESRCH", "EACCES", "EPERM"].includes(code ?? ""))
          throw error;
      }
    }
    assert.equal(
      matches.length,
      1,
      "expected exactly one PID for the exact packaged executable",
    );
    const pid = matches[0];
    assert.ok(pid !== undefined);
    const clients = query("clients");
    assert.ok(Array.isArray(clients), "invalid compositor clients response");
    const clientRecords = clients.map(record);
    const windows = clientRecords.filter((client) => client.pid === pid);
    assert.equal(
      windows.length,
      1,
      "expected exactly one compositor window for the packaged PID",
    );
    const target = windowIdentity(windows[0]);
    assert.equal(
      target.mapped,
      true,
      "packaged application window is not mapped",
    );
    assert.equal(
      clientRecords.filter((client) => client.address === target.address)
        .length,
      1,
      "ambiguous compositor window address",
    );
    const identity = { pid, address: target.address };
    if (owner)
      assert.deepEqual(
        identity,
        owner,
        "packaged PID/window ownership changed",
      );
    owner = identity;
    return identity;
  }

  function send(key: "Tab" | "Return"): DispatchTrace {
    if (blocked)
      throw new Error(
        "Trusted native keyboard blocked: previous ownership/input failure",
      );
    try {
      assert.ok(
        key === "Tab" || key === "Return",
        "only Tab and Return are permitted",
      );
      const target = verifiedOwner();
      const before = windowIdentity(query("activewindow"));
      assert.ok(
        !before.xwayland ||
          (before.pid === target.pid && before.address === target.address),
        "refusing input while a different XWayland window is active",
      );
      const trace: DispatchTrace = {
        cli,
        key,
        ...target,
        activeBefore: before,
        activeAfter: null,
        acknowledgement: null,
      };
      dispatches.push(trace);
      trace.acknowledgement = io
        .hyprctl(
          cli,
          ["dispatch", "sendshortcut", `, ${key}, address:${target.address}`],
          compositorEnv,
        )
        .trim();
      assert.equal(
        trace.acknowledgement,
        "ok",
        "compositor did not acknowledge scoped input",
      );
      trace.activeAfter = windowIdentity(query("activewindow"));
      assert.deepEqual(
        trace.activeAfter,
        before,
        "scoped input changed the active compositor window",
      );
      verifiedOwner();
      return trace;
    } catch (error) {
      blocked = true;
      throw new Error(
        `Trusted native keyboard blocked: ${error instanceof Error ? error.message : String(error)}`,
        { cause: error },
      );
    }
  }

  return { send, dispatches };
}
