import { spawn } from "node:child_process";
import {
  access,
  chmod,
  copyFile,
  mkdir,
  readdir,
  readFile,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(scriptDir, "..");
const targetRoot = path.join(appRoot, "src-tauri", "target");
const targetTriple = "x86_64-pc-windows-gnu";
const copyToDesktop = process.argv.includes("--copy-to-desktop");

const mingwPackages = [
  "binutils-mingw-w64-x86-64",
  "mingw-w64-common",
  "mingw-w64-x86-64-dev",
  "gcc-mingw-w64-base",
  "gcc-mingw-w64-x86-64-win32",
  "gcc-mingw-w64-x86-64-win32-runtime",
];
const nsisPackages = ["nsis", "nsis-common"];

const mingwDownloadDir = path.join(targetRoot, "mingw-download");
const mingwLocalDir = path.join(targetRoot, "mingw-local");
const nsisDownloadDir = path.join(targetRoot, "nsis-download");
const nsisLocalDir = path.join(targetRoot, "nsis-local");
const nsisDataDir = path.join(nsisLocalDir, "usr", "share", "nsis");
const nsisWrapperDir = path.join(targetRoot, "nsis-wrapper", "bin");
const tauriMetadataDir = path.join(targetRoot, "tauri-cross-metadata");
const releaseDir = path.join(targetRoot, targetTriple, "release");

await ensureHostTools();
await ensureDownloaded(mingwDownloadDir, mingwPackages);
await ensureExtracted(
  mingwDownloadDir,
  mingwLocalDir,
  path.join(mingwLocalDir, "usr", "share", "mingw-w64", "include"),
);
await ensureMingwAliases();
await ensureDownloaded(nsisDownloadDir, nsisPackages);
await ensureExtracted(
  nsisDownloadDir,
  nsisLocalDir,
  path.join(nsisDataDir, "Stubs", "zlib-x86-unicode"),
);
await writeMakensisWrapper();
await writeTauriCrossBundleMetadata();

const buildEnv = {
  ...process.env,
  AR_x86_64_pc_windows_gnu: "x86_64-w64-mingw32-ar",
  CC_x86_64_pc_windows_gnu: "x86_64-w64-mingw32-gcc",
  CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER: "x86_64-w64-mingw32-gcc",
  NSISDIR: nsisDataDir,
  PKG_CONFIG_PATH: [tauriMetadataDir, process.env.PKG_CONFIG_PATH]
    .filter(Boolean)
    .join(path.delimiter),
  TAURI_LINUX_AYATANA_APPINDICATOR: "0",
  PATH: [
    nsisWrapperDir,
    path.join(nsisLocalDir, "usr", "bin"),
    path.join(mingwLocalDir, "usr", "bin"),
    process.env.PATH,
  ]
    .filter(Boolean)
    .join(path.delimiter),
};

await rm(path.join(releaseDir, "bundle", "nsis"), { force: true, recursive: true });

const tauriResult = await run(
  "bunx",
  ["tauri", "build", "--bundles", "nsis", "--target", targetTriple],
  {
    cwd: appRoot,
    env: buildEnv,
    allowFailure: true,
  },
);

if (tauriResult.code !== 0) {
  await runManualNsis(buildEnv);
}

const artifactPath = await normalizeArtifact();
await verifyWindowsImports();
await run("bun", ["run", "scripts/verify-desktop-artifact.mjs", artifactPath, "exe"], {
  cwd: appRoot,
  env: process.env,
});

if (copyToDesktop) {
  const desktopPath = await windowsDesktopPath();
  const destination = path.join(desktopPath, "Cutokyo-Windows-Setup.exe");
  await copyFile(artifactPath, destination);
  const metadata = await stat(destination);
  console.log(`copied ${destination} (${metadata.size} bytes)`);
}

console.log(`built ${artifactPath}`);

async function ensureHostTools() {
  if (process.platform !== "linux") {
    throw new Error(
      "build-windows-local.mjs is for local Linux/WSL cross-builds. Use GitHub Actions or a native Windows host for release builds.",
    );
  }
  await Promise.all([
    ensureCommand("apt-get", ["--version"]),
    ensureCommand("dpkg-deb", ["--version"]),
  ]);
}

async function ensureCommand(command, args) {
  await run(command, args, { cwd: appRoot, env: process.env, quiet: true });
}

async function ensureDownloaded(downloadDir, packages) {
  await mkdir(downloadDir, { recursive: true });
  const entries = await readdir(downloadDir).catch(() => []);
  const missing = packages.filter(
    (packageName) =>
      !entries.some((entry) => entry.startsWith(`${packageName}_`) && entry.endsWith(".deb")),
  );
  if (missing.length === 0) {
    return;
  }
  await run("apt-get", ["download", ...missing], { cwd: downloadDir, env: process.env });
}

async function ensureExtracted(downloadDir, localDir, sentinelPath) {
  if (await exists(sentinelPath)) {
    return;
  }
  await mkdir(localDir, { recursive: true });
  const debs = (await readdir(downloadDir)).filter((entry) => entry.endsWith(".deb")).sort();
  if (debs.length === 0) {
    throw new Error(`No .deb packages found in ${downloadDir}`);
  }
  for (const deb of debs) {
    await run("dpkg-deb", ["-x", path.join(downloadDir, deb), localDir], {
      cwd: appRoot,
      env: process.env,
    });
  }
}

async function ensureMingwAliases() {
  const binDir = path.join(mingwLocalDir, "usr", "bin");
  const aliases = [
    ["x86_64-w64-mingw32-gcc-win32", "x86_64-w64-mingw32-gcc"],
    ["x86_64-w64-mingw32-cpp-win32", "x86_64-w64-mingw32-cpp"],
    ["x86_64-w64-mingw32-gcc-ar-win32", "x86_64-w64-mingw32-gcc-ar"],
    ["x86_64-w64-mingw32-gcc-nm-win32", "x86_64-w64-mingw32-gcc-nm"],
    ["x86_64-w64-mingw32-gcc-ranlib-win32", "x86_64-w64-mingw32-gcc-ranlib"],
  ];
  for (const [source, target] of aliases) {
    const sourcePath = path.join(binDir, source);
    if (!(await exists(sourcePath))) {
      throw new Error(`Missing MinGW tool ${sourcePath}`);
    }
    const targetPath = path.join(binDir, target);
    await rm(targetPath, { force: true });
    await symlink(source, targetPath);
  }
}

async function writeMakensisWrapper() {
  await mkdir(nsisWrapperDir, { recursive: true });
  const wrapperPath = path.join(nsisWrapperDir, "makensis");
  const makensisPath = path.join(nsisLocalDir, "usr", "bin", "makensis");
  await writeFile(
    wrapperPath,
    [
      "#!/usr/bin/env sh",
      `export NSISDIR=${shellQuote(nsisDataDir)}`,
      `exec ${shellQuote(makensisPath)} "$@"`,
      "",
    ].join("\n"),
    "utf8",
  );
  await chmod(wrapperPath, 0o755);
}

async function writeTauriCrossBundleMetadata() {
  // Tauri inspects the host tray library even while bundling a Windows target.
  // Supply inert host metadata so that cross-packaging can reach the NSIS stage.
  await mkdir(tauriMetadataDir, { recursive: true });
  await writeFile(path.join(tauriMetadataDir, "libappindicator3.so.1"), "", "utf8");
  await writeFile(
    path.join(tauriMetadataDir, "appindicator3-0.1.pc"),
    [
      `prefix=${tauriMetadataDir}`,
      "libdir=${prefix}",
      "",
      "Name: appindicator3",
      "Description: Cross-build-only Tauri host metadata",
      "Version: 0.1",
      "Libs: -L${libdir} -lappindicator3",
      "",
    ].join("\n"),
    "utf8",
  );
}

async function runManualNsis(env) {
  const nsisDir = path.join(releaseDir, "nsis", "x64");
  const installerScript = path.join(nsisDir, "installer.nsi");
  if (!(await exists(installerScript))) {
    throw new Error("Tauri build failed before generating the NSIS installer script.");
  }
  await run("makensis", ["installer.nsi"], { cwd: nsisDir, env });
}

async function normalizeArtifact() {
  const config = JSON.parse(
    await readFile(path.join(appRoot, "src-tauri", "tauri.conf.json"), "utf8"),
  );
  const bundleDir = path.join(releaseDir, "bundle", "nsis");
  const canonicalName = `${config.productName}_${config.version}_x64-setup.exe`;
  const canonicalPath = path.join(bundleDir, canonicalName);
  if (await exists(canonicalPath)) {
    return canonicalPath;
  }

  const manualOutput = path.join(releaseDir, "nsis", "x64", "nsis-output.exe");
  if (!(await exists(manualOutput))) {
    throw new Error(`No NSIS setup artifact found at ${canonicalPath} or ${manualOutput}`);
  }
  await mkdir(bundleDir, { recursive: true });
  await copyFile(manualOutput, canonicalPath);
  return canonicalPath;
}

async function verifyWindowsImports() {
  const binaryPath = path.join(releaseDir, "cutokyo.exe");
  const objdumpPath = path.join(mingwLocalDir, "usr", "bin", "x86_64-w64-mingw32-objdump");
  const result = await run(objdumpPath, ["-p", binaryPath], {
    cwd: appRoot,
    env: process.env,
    capture: true,
  });
  const importedDlls = [...result.stdout.matchAll(/DLL Name:\s+([^\r\n]+)/g)].map(
    (match) => match[1],
  );
  const blockedDlls = importedDlls.filter((dll) =>
    /lib(?:gcc|stdc\+\+|winpthread|unwind)/i.test(dll),
  );
  if (blockedDlls.length > 0) {
    throw new Error(`Windows build imports unbundled runtime DLLs: ${blockedDlls.join(", ")}`);
  }
  if (!importedDlls.some((dll) => dll.toLowerCase() === "webview2loader.dll")) {
    throw new Error(
      "Windows build does not import WebView2Loader.dll; verify Tauri WebView packaging before release.",
    );
  }
}

async function windowsDesktopPath() {
  if (process.env.CUTOKYO_WINDOWS_DESKTOP) {
    return process.env.CUTOKYO_WINDOWS_DESKTOP;
  }
  const candidate = "/mnt/c/Users/lucas/Desktop";
  if (await exists(candidate)) {
    return candidate;
  }
  throw new Error(
    "Could not find the Windows Desktop. Set CUTOKYO_WINDOWS_DESKTOP or omit --copy-to-desktop.",
  );
}

function run(command, args, options) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env,
      shell: process.platform === "win32",
      stdio: options.capture ? ["ignore", "pipe", "pipe"] : options.quiet ? "ignore" : "inherit",
    });

    const stdout = [];
    const stderr = [];
    if (options.capture) {
      child.stdout.on("data", (chunk) => stdout.push(chunk));
      child.stderr.on("data", (chunk) => stderr.push(chunk));
    }

    child.on("exit", (code, signal) => {
      const result = {
        code: code ?? 1,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      };
      if (code === 0 || options.allowFailure) {
        resolve(result);
        return;
      }
      reject(new Error(`${command} ${args.join(" ")} failed with ${signal ?? code}`));
    });
    child.on("error", reject);
  });
}

async function exists(filePath) {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

function shellQuote(value) {
  return `'${value.replaceAll("'", "'\\''")}'`;
}
