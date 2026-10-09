import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";

const [artifactArgument, extension] = process.argv.slice(2);
if (!artifactArgument || !extension) {
  throw new Error("Usage: test-desktop-artifact.mjs <artifact-path> <extension>");
}

const artifactPath = path.resolve(artifactArgument);
const tempRoot = await mkdtemp(path.join(os.tmpdir(), "cutokyo-artifact-test-"));
let detachDmg = null;
let uninstallLinuxPackage = null;
let uninstallWindows = null;
let systemActivationResult = null;
let activeUninstallResult = null;
let windowsCliPathInstalled = false;
let windowsCliPathRestored = false;
let windowsInstallRoot = null;
let windowsUserPathBefore = null;

try {
  await run(process.execPath, [
    path.join(import.meta.dirname, "verify-desktop-artifact.mjs"),
    artifactPath,
    extension,
  ]);
  const executable = await prepareArtifact();
  await assertExecutable(executable);
  await smokeEmergencyRestore(executable);
  await smokeProxyRuntime(executable, false);
  await smokeOccupiedCapturePort(executable);
  if (
    process.env.CUTOKYO_TEST_SYSTEM_ACTIVATION === "true" &&
    ["darwin", "win32"].includes(process.platform)
  ) {
    systemActivationResult = await smokeSystemActivationLifecycle(executable);
  }
  if (process.env.CUTOKYO_TEST_ACTIVE_UNINSTALL === "true" && process.platform === "win32") {
    activeUninstallResult = await smokeWindowsActiveUninstall(executable);
  }
  console.log(
    JSON.stringify(
      {
        artifact: artifactPath,
        executable,
        extension,
        nativePlatform: process.platform,
        ok: true,
        tests: [
          "package-structure",
          "emergency-restore",
          "proxy-runtime",
          "capture-port-collision",
          "api-port-collision",
          ...(systemActivationResult ? [systemActivationResult] : []),
          ...(activeUninstallResult ? [activeUninstallResult] : []),
          ...(windowsCliPathInstalled && windowsCliPathRestored
            ? ["cli-path-install-restore"]
            : []),
        ],
      },
      null,
      2,
    ),
  );
} finally {
  if (detachDmg) {
    await run("hdiutil", ["detach", detachDmg, "-force"], { allowFailure: true });
  }
  if (uninstallWindows) {
    await run(uninstallWindows, ["/S"], { allowFailure: true });
    await assertWindowsCliPathRestored();
  }
  if (uninstallLinuxPackage) {
    await run("sudo", ["dpkg", "--remove", uninstallLinuxPackage]);
    try {
      await access("/usr/bin/cutokyo");
      throw new Error("Debian uninstall left /usr/bin/cutokyo behind");
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }
  await removeTempRoot();
}

async function removeTempRoot() {
  const deadline = Date.now() + 10_000;
  while (true) {
    try {
      await rm(tempRoot, { force: true, recursive: true });
      return;
    } catch (error) {
      if (!["EBUSY", "ENOTEMPTY", "EPERM"].includes(error.code) || Date.now() >= deadline) {
        throw error;
      }
      await delay(250);
    }
  }
}

async function smokeWindowsActiveUninstall(executable) {
  if (!uninstallWindows) {
    throw new Error("Windows active-uninstall test could not find the NSIS uninstaller");
  }
  const before = await systemProxySnapshot();
  const backupPath = path.join(tempRoot, ".cutokyo", "capture", "system-proxy-backup.json");
  const runKey = "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
  const autostartBefore = await windowsAutostartSnapshot();
  let activeProcess = null;
  if (autostartBefore.exists) {
    throw new Error("Refusing active-uninstall test because Cutokyo autostart already exists");
  }
  try {
    await access(backupPath);
    throw new Error("Refusing active-uninstall test because a Cutokyo proxy backup already exists");
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }

  await mkdir(path.dirname(backupPath), { recursive: true });
  await writeFile(backupPath, `${before}\n`, { mode: 0o600 });
  const activateScript = [
    "$ErrorActionPreference = 'Stop';",
    "$path = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings';",
    "Set-ItemProperty -Path $path -Name ProxyEnable -Type DWord -Value 1;",
    "Set-ItemProperty -Path $path -Name ProxyServer -Type String -Value 'http=127.0.0.1:49322;https=127.0.0.1:49322';",
    "Set-ItemProperty -Path $path -Name ProxyOverride -Type String -Value 'localhost;127.0.0.1;<local>';",
    "[Environment]::SetEnvironmentVariable('HTTP_PROXY', 'http://127.0.0.1:49322', 'User');",
    "[Environment]::SetEnvironmentVariable('HTTPS_PROXY', 'http://127.0.0.1:49322', 'User');",
    "[Environment]::SetEnvironmentVariable('ALL_PROXY', 'http://127.0.0.1:49322', 'User');",
    "[Environment]::SetEnvironmentVariable('NO_PROXY', 'localhost,127.0.0.1', 'User');",
    `New-Item -Path '${runKey}' -Force | Out-Null;`,
    `Set-ItemProperty -Path '${runKey}' -Name Cutokyo -Type String -Value $env:CUTOKYO_INSTALLED_EXE;`,
  ].join(" ");

  try {
    const apiPort = await freePort();
    activeProcess = spawn(executable, [], {
      env: {
        ...process.env,
        CUTOKYO_API_ADDR: `127.0.0.1:${apiPort}`,
        CUTOKYO_PROXY_ONLY: "1",
        HOME: tempRoot,
        USERPROFILE: tempRoot,
      },
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    await waitForJson(`http://127.0.0.1:${apiPort}/cutokyo/status`, activeProcess);
    await postActivation(apiPort, true, true);

    await run("powershell.exe", ["-NoProfile", "-Command", activateScript], {
      env: { ...process.env, CUTOKYO_INSTALLED_EXE: executable },
    });
    await assertSystemProxyActive();

    const uninstaller = uninstallWindows;
    await run(uninstaller, ["/S"], {
      env: { ...process.env, HOME: tempRoot, USERPROFILE: tempRoot },
    });
    await waitForWindowsUninstall(executable, backupPath, before, activeProcess);
    await assertWindowsCliPathRestored();
    uninstallWindows = null;
    await run(nativeCurl(), [
      "--fail",
      "--silent",
      "--show-error",
      "--max-time",
      "20",
      "https://example.com/",
      "--output",
      nullDevice(),
    ]);
    return "active-uninstall-restore";
  } finally {
    if (activeProcess) {
      await terminate(activeProcess);
    }
    if ((await systemProxySnapshot()) !== before) {
      await restoreWindowsProxySnapshot(before);
    }
    await run(
      "powershell.exe",
      [
        "-NoProfile",
        "-Command",
        `Remove-ItemProperty -Path '${runKey}' -Name Cutokyo -ErrorAction SilentlyContinue`,
      ],
      { allowFailure: true },
    );
    await rm(backupPath, { force: true });
  }
}

async function waitForWindowsUninstall(executable, backupPath, expectedProxy, activeProcess) {
  const deadline = Date.now() + 30_000;
  let state;
  while (Date.now() < deadline) {
    state = {
      autostart: await windowsAutostartSnapshot(),
      backupExists: await pathExists(backupPath),
      executableExists: await pathExists(executable),
      installEntries: await readdir(path.dirname(executable)).catch(() => []),
      processExited: activeProcess.exitCode !== null,
      processes: await windowsCutokyoProcesses(),
      proxy: await systemProxySnapshot(),
      recoveryReport: await readFile(
        path.join(path.dirname(executable), "cutokyo-recovery-report.txt"),
        "utf8",
      ).catch(() => null),
    };
    if (
      state.proxy === expectedProxy &&
      !state.autostart.exists &&
      !state.backupExists &&
      !state.executableExists &&
      state.processExited &&
      state.processes.length === 0
    ) {
      return;
    }
    await delay(250);
  }
  throw new Error(
    `NSIS uninstall did not complete recovery within 30 seconds:\nexpectedProxy=${expectedProxy}\nstate=${JSON.stringify(state)}`,
  );
}

async function windowsCutokyoProcesses() {
  const script = [
    "$items = @(Get-CimInstance Win32_Process -Filter \"Name = 'cutokyo.exe'\" | Select-Object ProcessId,ParentProcessId,ExecutablePath,CommandLine);",
    "[ordered]@{ processes = $items } | ConvertTo-Json -Depth 3 -Compress;",
  ].join(" ");
  const result = await run("powershell.exe", ["-NoProfile", "-Command", script], {
    quiet: true,
  });
  return JSON.parse(result.stdout).processes;
}

async function pathExists(filePath) {
  try {
    await access(filePath);
    return true;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

async function windowsAutostartSnapshot() {
  const script = [
    "$path = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run';",
    "$item = Get-ItemProperty -Path $path -Name Cutokyo -ErrorAction SilentlyContinue;",
    "[ordered]@{ exists = ($null -ne $item); value = if ($null -ne $item) { [string]$item.Cutokyo } else { $null } } | ConvertTo-Json -Compress;",
  ].join(" ");
  const result = await run("powershell.exe", ["-NoProfile", "-Command", script], {
    quiet: true,
  });
  return JSON.parse(result.stdout);
}

async function restoreWindowsProxySnapshot(snapshot) {
  const script = [
    "$ErrorActionPreference = 'Stop';",
    "$item = $env:CUTOKYO_PROXY_SNAPSHOT | ConvertFrom-Json;",
    "$path = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings';",
    "if ($null -ne $item.ProxyEnable) { Set-ItemProperty -Path $path -Name ProxyEnable -Type DWord -Value ([int]$item.ProxyEnable) };",
    "if ($null -ne $item.ProxyServer) { Set-ItemProperty -Path $path -Name ProxyServer -Type String -Value ([string]$item.ProxyServer) } else { Remove-ItemProperty -Path $path -Name ProxyServer -ErrorAction SilentlyContinue };",
    "if ($null -ne $item.ProxyOverride) { Set-ItemProperty -Path $path -Name ProxyOverride -Type String -Value ([string]$item.ProxyOverride) } else { Remove-ItemProperty -Path $path -Name ProxyOverride -ErrorAction SilentlyContinue };",
    "foreach ($name in @('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY')) { if ($item.PSObject.Properties.Name -contains $name -and $null -ne $item.$name) { [Environment]::SetEnvironmentVariable($name, [string]$item.$name, 'User') } else { [Environment]::SetEnvironmentVariable($name, $null, 'User') } };",
    'Add-Type -TypeDefinition \'using System; using System.Runtime.InteropServices; public static class CutokyoAuditWinInet { [DllImport("wininet.dll", SetLastError = true)] public static extern bool InternetSetOption(IntPtr hInternet, int option, IntPtr buffer, int length); } public static class CutokyoAuditUser32 { [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, UIntPtr wParam, string lParam, uint flags, uint timeout, out UIntPtr result); }\';',
    "[CutokyoAuditWinInet]::InternetSetOption([IntPtr]::Zero, 39, [IntPtr]::Zero, 0) | Out-Null;",
    "[CutokyoAuditWinInet]::InternetSetOption([IntPtr]::Zero, 37, [IntPtr]::Zero, 0) | Out-Null;",
    "$refreshResult = [UIntPtr]::Zero;",
    "[CutokyoAuditUser32]::SendMessageTimeout([IntPtr]0xffff, 0x1a, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$refreshResult) | Out-Null;",
  ].join(" ");
  await run("powershell.exe", ["-NoProfile", "-Command", script], {
    env: { ...process.env, CUTOKYO_PROXY_SNAPSHOT: snapshot },
  });
}

async function smokeEmergencyRestore(executable) {
  const codexPath = path.join(tempRoot, ".codex", "config.toml");
  const claudePath = path.join(tempRoot, ".claude", "settings.json");
  const geminiPath = path.join(tempRoot, ".gemini", "settings.json");
  const envPath = path.join(tempRoot, ".env");
  const autostartPath =
    process.platform === "linux"
      ? path.join(tempRoot, ".config", "autostart", "Cutokyo.desktop")
      : process.platform === "darwin"
        ? path.join(tempRoot, "Library", "LaunchAgents", "Cutokyo.plist")
        : null;
  await Promise.all([
    mkdir(path.dirname(codexPath), { recursive: true }),
    mkdir(path.dirname(claudePath), { recursive: true }),
    mkdir(path.dirname(geminiPath), { recursive: true }),
    ...(autostartPath ? [mkdir(path.dirname(autostartPath), { recursive: true })] : []),
  ]);
  if (autostartPath) {
    await writeFile(autostartPath, "cutokyo-owned-autostart\n");
  }
  await writeFile(
    codexPath,
    [
      'model = "gpt-5"',
      'model_provider = "cutokyo"',
      "# >>> Cutokyo Codex provider",
      "[model_providers.cutokyo]",
      'name = "Cutokyo"',
      'base_url = "http://localhost:49321/v1"',
      'wire_api = "responses"',
      "# <<< Cutokyo Codex provider",
      "",
    ].join("\n"),
  );
  await writeFile(
    claudePath,
    `${JSON.stringify({ env: { ANTHROPIC_BASE_URL: "http://localhost:49321", KEEP: "yes" }, unrelated: true }, null, 2)}\n`,
  );
  await writeFile(
    geminiPath,
    `${JSON.stringify({ security: { auth: { keep: true, selectedType: "gateway", useExternal: true } }, unrelated: true }, null, 2)}\n`,
  );
  await writeFile(
    envPath,
    [
      "KEEP=yes",
      "# >>> Cutokyo Gemini gateway",
      "GOOGLE_GEMINI_BASE_URL=http://localhost:49321",
      "# <<< Cutokyo Gemini gateway",
      "",
    ].join("\n"),
  );

  const result = await run(executable, ["--emergency-restore"], {
    env: { ...process.env, HOME: tempRoot, USERPROFILE: tempRoot },
  });
  const report = JSON.parse(result.stdout);
  if (report.ok !== true) {
    throw new Error(`Emergency restore did not report success: ${result.stdout}`);
  }
  const repeatedResult = await run(executable, ["--emergency-restore"], {
    env: { ...process.env, HOME: tempRoot, USERPROFILE: tempRoot },
  });
  const repeatedReport = JSON.parse(repeatedResult.stdout);
  if (repeatedReport.ok !== true) {
    throw new Error(`Repeated emergency restore did not report success: ${repeatedResult.stdout}`);
  }

  const [codex, claudeText, geminiText, envText] = await Promise.all([
    readFile(codexPath, "utf8"),
    readFile(claudePath, "utf8"),
    readFile(geminiPath, "utf8"),
    readFile(envPath, "utf8"),
  ]);
  const claude = JSON.parse(claudeText);
  const gemini = JSON.parse(geminiText);
  if (
    codex.includes("cutokyo") ||
    !codex.includes('model = "gpt-5"') ||
    claude.env.ANTHROPIC_BASE_URL !== undefined ||
    claude.env.KEEP !== "yes" ||
    gemini.security.auth.selectedType !== undefined ||
    gemini.security.auth.useExternal !== undefined ||
    gemini.security.auth.keep !== true ||
    !envText.includes("KEEP=yes") ||
    envText.includes("Cutokyo Gemini gateway")
  ) {
    throw new Error("Emergency restore did not preserve unrelated harness configuration");
  }
  if (autostartPath) {
    try {
      await access(autostartPath);
      throw new Error("Emergency restore left Cutokyo autostart enabled");
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }
}

async function prepareArtifact() {
  if (extension === "deb") {
    requirePlatform("linux");
    await run("dpkg-deb", ["--info", artifactPath]);
    await run("dpkg-deb", ["--contents", artifactPath]);
    if (process.env.CUTOKYO_TEST_NATIVE_INSTALL === "true") {
      await run("sudo", ["dpkg", "--install", artifactPath]);
      uninstallLinuxPackage = "cutokyo";
      return "/usr/bin/cutokyo";
    }
    const root = path.join(tempRoot, "deb-root");
    await run("dpkg-deb", ["-x", artifactPath, root]);
    return findFile(root, (entry) => entry === "cutokyo");
  }
  if (extension === "AppImage") {
    requirePlatform("linux");
    await chmod(artifactPath, 0o755);
    const extractRoot = path.join(tempRoot, "appimage");
    await mkdir(extractRoot, { recursive: true });
    await run(artifactPath, ["--appimage-extract"], { cwd: extractRoot });
    return findFile(
      path.join(extractRoot, "squashfs-root"),
      (entry, parent) => entry === "cutokyo" && parent.endsWith(path.join("usr", "bin")),
    );
  }
  if (extension === "exe") {
    requirePlatform("win32");
    await verifyWindowsSignature();
    const installRoot = path.join(tempRoot, "windows-install");
    windowsInstallRoot = installRoot;
    windowsUserPathBefore = await windowsUserPath();
    await run(artifactPath, ["/S", `/D=${installRoot}`]);
    await assertWindowsCliPathInstalled();
    uninstallWindows = await findFile(
      installRoot,
      (entry) => /^uninstall.*\.exe$/i.test(entry),
      false,
    );
    const executable = await findFile(
      installRoot,
      (entry) => entry.toLowerCase() === "cutokyo.exe",
    );
    await assertWindowsX64Executable(executable);
    return executable;
  }
  if (extension === "dmg") {
    requirePlatform("darwin");
    await run("hdiutil", ["verify", artifactPath]);
    const mountPoint = path.join(tempRoot, "dmg");
    await run("mkdir", ["-p", mountPoint]);
    await run("hdiutil", [
      "attach",
      artifactPath,
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mountPoint,
    ]);
    detachDmg = mountPoint;
    const mountedApp = await findFile(mountPoint, (entry) => entry.endsWith(".app"));
    const applications = path.join(tempRoot, "Applications");
    await mkdir(applications, { recursive: true });
    const app = path.join(applications, path.basename(mountedApp));
    await run("ditto", [mountedApp, app]);
    await verifyMacSignature(app);
    const executable = await findFile(
      path.join(app, "Contents", "MacOS"),
      (entry) => entry === "cutokyo" || entry === "Cutokyo",
    );
    await run("file", [executable]);
    await run("lipo", ["-archs", executable]);
    return executable;
  }
  throw new Error(`Unsupported artifact extension: ${extension}`);
}

async function assertWindowsCliPathInstalled() {
  const current = await windowsUserPath();
  if (
    !windowsInstallRoot ||
    !windowsPathEntries(current).includes(normalizeWindowsPath(windowsInstallRoot))
  ) {
    throw new Error(`Windows setup did not add ${windowsInstallRoot} to the user PATH`);
  }
  windowsCliPathInstalled = true;
}

async function assertWindowsCliPathRestored() {
  if (process.platform !== "win32" || windowsUserPathBefore === null) return;
  const current = await windowsUserPath();
  if (current !== windowsUserPathBefore) {
    throw new Error("Windows uninstall did not restore the exact prior user PATH");
  }
  windowsCliPathRestored = true;
}

async function windowsUserPath() {
  const result = await run(
    "powershell.exe",
    [
      "-NoProfile",
      "-Command",
      "[Console]::Out.Write([Environment]::GetEnvironmentVariable('Path', 'User'))",
    ],
    { capture: true },
  );
  return result.stdout;
}

function windowsPathEntries(value) {
  return value.split(";").map(normalizeWindowsPath).filter(Boolean);
}

function normalizeWindowsPath(value) {
  const unquoted = value.trim().replace(/^"|"$/g, "");
  return path.win32
    .normalize(unquoted)
    .replace(/[\\/]+$/, "")
    .toLowerCase();
}

async function smokeProxyRuntime(executable, expectCollision) {
  const apiPort = await freePort();
  const child = spawn(executable, [], {
    env: {
      ...process.env,
      CUTOKYO_API_ADDR: `127.0.0.1:${apiPort}`,
      CUTOKYO_PROXY_ONLY: "1",
      HOME: tempRoot,
      USERPROFILE: tempRoot,
    },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const stdout = [];
  const stderr = [];
  child.stdout?.on("data", (chunk) => stdout.push(chunk));
  child.stderr?.on("data", (chunk) => stderr.push(chunk));
  try {
    const status = await waitForJson(`http://127.0.0.1:${apiPort}/cutokyo/status`, child);
    if (status.status !== "ready") {
      throw new Error(`Unexpected Cutokyo status: ${JSON.stringify(status)}`);
    }
    if (!expectCollision) {
      await assertPortAccepts(49322);
    }
    return { apiPort, child };
  } catch (error) {
    throw new Error(
      `${error.message}\nstdout=${Buffer.concat(stdout).toString("utf8")}\nstderr=${Buffer.concat(stderr).toString("utf8")}`,
    );
  } finally {
    await terminate(child);
  }
}

async function smokeOccupiedCapturePort(executable) {
  const blocker = net.createServer((socket) => socket.destroy());
  await new Promise((resolve, reject) => {
    blocker.once("error", reject);
    blocker.listen(49322, "127.0.0.1", resolve);
  });
  const apiPort = await freePort();
  const child = spawn(executable, [], {
    env: {
      ...process.env,
      CUTOKYO_API_ADDR: `127.0.0.1:${apiPort}`,
      CUTOKYO_PROXY_ONLY: "1",
      HOME: tempRoot,
      USERPROFILE: tempRoot,
    },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const stderr = [];
  child.stderr?.on("data", (chunk) => stderr.push(chunk));
  try {
    const exit = await waitForExit(child, 5_000);
    const error = Buffer.concat(stderr).toString("utf8");
    if (exit === 0 || !error.includes("capture proxy could not bind")) {
      throw new Error(
        `Cutokyo did not fail safely when capture port 49322 was occupied: exit=${exit} stderr=${error}`,
      );
    }
    await assertPortFree(apiPort);
  } finally {
    await terminate(child);
    await new Promise((resolve) => blocker.close(resolve));
  }
  await smokeOccupiedApiPort(executable);
}

async function smokeOccupiedApiPort(executable) {
  const blocker = net.createServer((socket) => socket.destroy());
  await new Promise((resolve, reject) => {
    blocker.once("error", reject);
    blocker.listen(49321, "127.0.0.1", resolve);
  });
  const child = spawn(executable, [], {
    env: {
      ...process.env,
      CUTOKYO_PROXY_ONLY: "1",
      HOME: tempRoot,
      USERPROFILE: tempRoot,
    },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const stderr = [];
  child.stderr?.on("data", (chunk) => stderr.push(chunk));
  try {
    const exit = await waitForExit(child, 5_000);
    const error = Buffer.concat(stderr).toString("utf8");
    if (exit === 0 || !error.includes("local API could not bind")) {
      throw new Error(
        `Cutokyo did not fail safely when API port 49321 was occupied: exit=${exit} stderr=${error}`,
      );
    }
    await assertPortFree(49322);
  } finally {
    await terminate(child);
    await new Promise((resolve) => blocker.close(resolve));
  }
}

async function smokeSystemActivationLifecycle(executable) {
  const apiPort = await freePort();
  const child = spawn(executable, [], {
    env: {
      ...process.env,
      CUTOKYO_API_ADDR: `127.0.0.1:${apiPort}`,
      CUTOKYO_PROXY_ONLY: "1",
      HOME: tempRoot,
      USERPROFILE: tempRoot,
    },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const stdout = [];
  const stderr = [];
  child.stdout?.on("data", (chunk) => stdout.push(chunk));
  child.stderr?.on("data", (chunk) => stderr.push(chunk));
  const before = await systemProxySnapshot();
  let activated = false;
  let fingerprint = null;
  let interactiveAuthorizationRequired = false;
  try {
    await waitForJson(`http://127.0.0.1:${apiPort}/cutokyo/status`, child);
    const activation = await postActivation(apiPort, true);
    activated = true;
    const certificatePath = activation.capture?.certificatePath;
    if (!certificatePath) {
      throw new Error(
        `Activation did not return a certificate path: ${JSON.stringify(activation)}`,
      );
    }
    fingerprint = createHash("sha1")
      .update(await readFile(certificatePath))
      .digest("hex")
      .toUpperCase();
    await assertSystemProxyActive();
    await assertCertificateTrust(fingerprint, true);
    await run(nativeCurl(), [
      "--fail",
      "--silent",
      "--show-error",
      "--max-time",
      "20",
      "--proxy",
      "http://127.0.0.1:49322",
      "https://example.com/",
      "--output",
      nullDevice(),
    ]);
    await postActivation(apiPort, false);
    activated = false;
  } catch (error) {
    if (error.message.includes("timed out")) {
      interactiveAuthorizationRequired = true;
      const certificatePath = path.join(tempRoot, ".cutokyo", "capture", "cutokyo-root-ca.der");
      try {
        fingerprint = createHash("sha1")
          .update(await readFile(certificatePath))
          .digest("hex")
          .toUpperCase();
      } catch (readError) {
        if (readError.code !== "ENOENT") throw readError;
      }
    } else {
      throw new Error(
        `${error.message}\nstdout=${Buffer.concat(stdout).toString("utf8")}\nstderr=${Buffer.concat(stderr).toString("utf8")}`,
      );
    }
  } finally {
    if (activated) {
      await postActivation(apiPort, false).catch(() => undefined);
    }
    await terminate(child);
  }
  const after = await systemProxySnapshot();
  if (after !== before) {
    throw new Error("System proxy settings were not restored byte-for-byte after deactivation");
  }
  if (fingerprint) {
    await assertCertificateTrust(fingerprint, false);
  }
  return interactiveAuthorizationRequired
    ? "system-activation-safely-requires-interactive-authorization"
    : "system-proxy-ca-activate-restore";
}

async function postActivation(apiPort, enabled, gatewayFallback = false) {
  const response = await fetch(`http://127.0.0.1:${apiPort}/cutokyo/activation`, {
    signal: AbortSignal.timeout(50_000),
    body: JSON.stringify({ applyChanges: true, clients: [], enabled, gatewayFallback }),
    headers: { "content-type": "application/json" },
    method: "POST",
  });
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`Activation HTTP ${response.status}: ${body}`);
  }
  return JSON.parse(body);
}

async function systemProxySnapshot() {
  if (process.platform === "win32") {
    const script = [
      "$path = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings';",
      "$item = Get-ItemProperty -Path $path;",
      "$property = { param($name) if ($item.PSObject.Properties.Name -contains $name) { $item.$name } else { $null } };",
      "[ordered]@{",
      "ProxyEnable = & $property 'ProxyEnable'; ProxyServer = & $property 'ProxyServer'; ProxyOverride = & $property 'ProxyOverride';",
      "HTTP_PROXY = [Environment]::GetEnvironmentVariable('HTTP_PROXY', 'User');",
      "HTTPS_PROXY = [Environment]::GetEnvironmentVariable('HTTPS_PROXY', 'User');",
      "ALL_PROXY = [Environment]::GetEnvironmentVariable('ALL_PROXY', 'User');",
      "NO_PROXY = [Environment]::GetEnvironmentVariable('NO_PROXY', 'User')",
      "} | ConvertTo-Json -Compress",
    ].join(" ");
    return (
      await run("powershell.exe", ["-NoProfile", "-Command", script], { quiet: true })
    ).stdout.trim();
  }
  const servicesOutput = await run("networksetup", ["-listallnetworkservices"], { quiet: true });
  const services = servicesOutput.stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("An asterisk") && !line.startsWith("*"));
  const snapshots = [];
  for (const service of services) {
    const web = await run("networksetup", ["-getwebproxy", service], { quiet: true });
    const secure = await run("networksetup", ["-getsecurewebproxy", service], { quiet: true });
    const bypass = await run("networksetup", ["-getproxybypassdomains", service], { quiet: true });
    snapshots.push([service, web.stdout, secure.stdout, bypass.stdout].join("\n"));
  }
  return snapshots.join("\n---\n").trim();
}

async function assertSystemProxyActive() {
  const snapshot = await systemProxySnapshot();
  if (process.platform === "win32") {
    const settings = JSON.parse(snapshot);
    if (Number(settings.ProxyEnable) !== 1 || !settings.ProxyServer?.includes("49322")) {
      throw new Error(`Windows proxy was not activated: ${snapshot}`);
    }
    return;
  }
  if (!snapshot.includes("127.0.0.1") || !snapshot.includes("49322")) {
    throw new Error("macOS network services were not routed to Cutokyo");
  }
}

async function assertCertificateTrust(fingerprint, expected) {
  const result =
    process.platform === "win32"
      ? await run("certutil", ["-user", "-store", "Root", fingerprint], {
          allowFailure: true,
          quiet: true,
        })
      : await run(
          "security",
          ["find-certificate", "-a", "-Z", "/Library/Keychains/System.keychain"],
          { allowFailure: true, quiet: true },
        );
  const found = `${result.stdout}\n${result.stderr}`.toUpperCase().includes(fingerprint);
  if (found !== expected) {
    throw new Error(`Certificate trust expected=${expected}, found=${found}`);
  }
}

function nativeCurl() {
  return process.platform === "win32" ? "curl.exe" : "/usr/bin/curl";
}

function nullDevice() {
  return process.platform === "win32" ? "NUL" : "/dev/null";
}

async function waitForExit(child, timeoutMs) {
  const exit = await Promise.race([
    new Promise((resolve) => child.once("exit", (code) => resolve(code ?? 1))),
    delay(timeoutMs).then(() => null),
  ]);
  if (exit === null) {
    throw new Error(`Cutokyo did not exit within ${timeoutMs}ms`);
  }
  return exit;
}

async function verifyWindowsSignature() {
  const requireSigned = process.env.CUTOKYO_REQUIRE_SIGNED_ARTIFACTS === "true";
  if (!requireSigned) return;
  const command = [
    "$ErrorActionPreference = 'Stop';",
    "$signature = Get-AuthenticodeSignature -LiteralPath $env:CUTOKYO_ARTIFACT;",
    "$signature | Select-Object Status,StatusMessage,SignerCertificate | ConvertTo-Json -Depth 3;",
    "if ($signature.Status -ne 'Valid') { exit 9 }",
  ].join(" ");
  const windowsPowerShell = path.join(
    process.env.SystemRoot ?? "C:\\Windows",
    "System32",
    "WindowsPowerShell",
    "v1.0",
    "powershell.exe",
  );
  await run(windowsPowerShell, ["-NoProfile", "-Command", command], {
    env: { ...process.env, CUTOKYO_ARTIFACT: artifactPath },
  });
}

async function verifyMacSignature(app) {
  const requireSigned = process.env.CUTOKYO_REQUIRE_SIGNED_ARTIFACTS === "true";
  const result = await run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", app], {
    allowFailure: !requireSigned,
  });
  if (requireSigned && result.code !== 0) {
    throw new Error("macOS application signature is not valid");
  }
  if (requireSigned) {
    await run("spctl", ["--assess", "--type", "execute", "--verbose=4", app]);
  }
}

async function assertExecutable(executable) {
  const metadata = await stat(executable);
  if (!metadata.isFile()) {
    throw new Error(`Expected installed executable file, got ${executable}`);
  }
}

async function assertWindowsX64Executable(executable) {
  const bytes = await readFile(executable);
  if (bytes.length < 64 || bytes[0] !== 0x4d || bytes[1] !== 0x5a) {
    throw new Error(`Installed Windows executable is not a PE file: ${executable}`);
  }
  const peOffset = bytes.readUInt32LE(0x3c);
  const signature = bytes.subarray(peOffset, peOffset + 6);
  if (
    signature.subarray(0, 4).toString("binary") !== "PE\u0000\u0000" ||
    signature.readUInt16LE(4) !== 0x8664
  ) {
    throw new Error(`Installed Windows executable is not x86_64: ${executable}`);
  }
}

async function findFile(root, predicate, required = true) {
  const queue = [root];
  while (queue.length > 0) {
    const current = queue.shift();
    const entries = await readdir(current, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = path.join(current, entry.name);
      if (
        predicate(entry.name, current, entry) &&
        (entry.isFile() || entry.name.endsWith(".app"))
      ) {
        return fullPath;
      }
      if (entry.isDirectory() && !entry.name.endsWith(".app")) {
        queue.push(fullPath);
      }
    }
  }
  if (required) {
    throw new Error(`No matching file found under ${root}`);
  }
  return null;
}

async function waitForJson(url, child) {
  const deadline = Date.now() + 20_000;
  let lastError;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`Cutokyo exited before readiness with code ${child.exitCode}`);
    }
    try {
      const response = await fetch(url);
      if (response.ok) {
        return response.json();
      }
      lastError = new Error(`HTTP ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await delay(200);
  }
  throw lastError ?? new Error(`Timed out waiting for ${url}`);
}

async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : null;
  await new Promise((resolve) => server.close(resolve));
  if (!port) {
    throw new Error("Could not reserve a local test port");
  }
  return port;
}

function assertPortAccepts(port) {
  return new Promise((resolve, reject) => {
    const socket = net.connect(port, "127.0.0.1");
    socket.setTimeout(2_000);
    socket.once("connect", () => {
      socket.destroy();
      resolve();
    });
    socket.once("timeout", () => {
      socket.destroy();
      reject(new Error(`Port ${port} did not accept a connection`));
    });
    socket.once("error", reject);
  });
}

function assertPortFree(port) {
  return new Promise((resolve, reject) => {
    const socket = net.connect(port, "127.0.0.1");
    socket.setTimeout(500);
    socket.once("connect", () => {
      socket.destroy();
      reject(new Error(`Port ${port} was unexpectedly left listening`));
    });
    socket.once("timeout", () => {
      socket.destroy();
      resolve();
    });
    socket.once("error", () => resolve());
  });
}

async function terminate(child) {
  if (child.exitCode !== null) {
    return;
  }
  if (process.platform === "win32") {
    await run("taskkill", ["/PID", String(child.pid), "/T", "/F"], { allowFailure: true });
  } else {
    child.kill("SIGTERM");
  }
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    delay(5_000).then(() => child.kill("SIGKILL")),
  ]);
}

function requirePlatform(platform) {
  if (process.platform !== platform) {
    throw new Error(
      `.${extension} native test requires ${platform}, current platform is ${process.platform}`,
    );
  }
}

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? process.cwd(),
      env: options.env ?? process.env,
      shell: false,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      const result = {
        code: code ?? 1,
        signal,
        stderr: Buffer.concat(stderr).toString("utf8"),
        stdout: Buffer.concat(stdout).toString("utf8"),
      };
      if (result.stdout && !options.quiet) process.stdout.write(result.stdout);
      if (result.stderr && !options.quiet) process.stderr.write(result.stderr);
      if (code === 0 || options.allowFailure) {
        resolve(result);
      } else {
        reject(
          new Error(`${command} ${args.join(" ")} failed with ${signal ?? code}: ${result.stderr}`),
        );
      }
    });
  });
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
