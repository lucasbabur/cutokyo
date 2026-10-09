import { execFileSync, spawn } from "node:child_process";
import { mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

import lighthouse from "lighthouse";

const CHROME_PATH =
  process.env.CHROME_PATH ?? "/mnt/c/Program Files/Google/Chrome/Application/chrome.exe";
const LIGHTHOUSE_OUTPUT_DIR = path.resolve(".lighthouseci");
const LOCAL_BASE_URL = `http://localhost:${process.env.LIGHTHOUSE_PORT ?? "3100"}`;
const SCORE_THRESHOLDS = {
  accessibility: 0.95,
  "best-practices": 0.9,
  performance: 0.8,
  seo: 0.9,
};

function execGit(args) {
  return execFileSync("git", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
  }).trim();
}

function getBaseRef() {
  try {
    return execGit(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]);
  } catch {
    try {
      execGit(["rev-parse", "HEAD~1"]);
      return "HEAD~1";
    } catch {
      return null;
    }
  }
}

function getChangedFiles(baseRef) {
  if (!baseRef) {
    return [];
  }

  return execGit(["diff", "--name-only", `${baseRef}...HEAD`])
    .split("\n")
    .map(normalizeFrontendPath)
    .filter(Boolean);
}

function getWorkingTreeChangedFiles() {
  try {
    return execGit(["status", "--short", "--porcelain=v1"])
      .split("\n")
      .map((line) => line.slice(3).trim())
      .map((filePath) => filePath.split(" -> ").at(-1) ?? filePath)
      .map(normalizeFrontendPath)
      .filter(Boolean);
  } catch {
    return [];
  }
}

function normalizeFrontendPath(filePath) {
  const normalized = filePath.trim().replaceAll("\\", "/");
  return normalized.startsWith("front-end/") ? normalized.slice("front-end/".length) : normalized;
}

function shouldSweepAllRoutes(baseRef, changedFiles) {
  if (!baseRef) {
    return true;
  }

  return changedFiles.some((filePath) => {
    return (
      /^src\/(entities|features|infrastructure|shared)\//.test(filePath) ||
      /^src\/app\/(?!.*page\.tsx$).+\.(ts|tsx)$/.test(filePath)
    );
  });
}

function routeFromPageFile(filePath) {
  const routePath = filePath
    .replace(/^src\/app\//, "")
    .replace(/\/page\.tsx$/, "")
    .replace(/^page\.tsx$/, "");

  const segments = routePath
    .split("/")
    .filter(Boolean)
    .filter((segment) => !(segment.startsWith("(") && segment.endsWith(")")));

  if (segments.some((segment) => segment.includes("["))) {
    return null;
  }

  return segments.length === 0 ? "/" : `/${segments.join("/")}`;
}

async function readPageFiles(directoryPath) {
  const entries = await readdir(directoryPath, { withFileTypes: true });
  const files = [];

  for (const entry of entries) {
    const entryPath = path.join(directoryPath, entry.name);

    if (entry.isDirectory()) {
      files.push(...(await readPageFiles(entryPath)));
      continue;
    }

    if (entry.isFile() && entry.name === "page.tsx") {
      files.push(path.relative(path.resolve(), entryPath).replace(/\\/g, "/"));
    }
  }

  return files;
}

async function waitForHttp(url) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      const response = await fetch(url);

      if (response.ok || response.status < 500) {
        return;
      }
    } catch {
      await delay(500);
    }
  }

  throw new Error(`Timed out waiting for ${url}.`);
}

async function waitForChromeLaunch(chrome) {
  return new Promise((resolve, reject) => {
    let logs = "";

    const onData = (chunk) => {
      logs += chunk.toString();

      const match = logs.match(/DevTools listening on ws:\/\/[^:]+:(\d+)\//);

      if (!match) {
        return;
      }

      cleanup();
      resolve(Number(match[1]));
    };

    const onError = (error) => {
      cleanup();
      reject(error);
    };

    const onExit = () => {
      cleanup();
      reject(new Error(`Chrome exited before exposing DevTools.\n${logs}`));
    };

    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`Timed out waiting for Chrome DevTools.\n${logs}`));
    }, 30_000);

    const cleanup = () => {
      clearTimeout(timeout);
      chrome.off("error", onError);
      chrome.off("exit", onExit);
      chrome.stderr.off("data", onData);
      chrome.stdout.off("data", onData);
    };

    chrome.once("error", onError);
    chrome.once("exit", onExit);
    chrome.stderr.on("data", onData);
    chrome.stdout.on("data", onData);
  });
}

function getReportContent(report) {
  if (Array.isArray(report)) {
    return report[0] ?? "";
  }

  return report;
}

function getRouteSlug(route) {
  return route === "/" ? "home" : route.slice(1).replace(/\//g, "--");
}

function quoteShellArg(value) {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}

async function launchChrome() {
  const userDataDir = await mkdtemp(path.join(os.tmpdir(), "company-front-lighthouse-"));
  const chromeCommand = [
    CHROME_PATH,
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    "--remote-debugging-address=localhost",
    "--remote-debugging-port=0",
    `--user-data-dir=${userDataDir}`,
    "about:blank",
  ]
    .map(quoteShellArg)
    .join(" ");
  const chrome = spawn("bash", ["-lc", `exec ${chromeCommand}`], {
    stdio: ["ignore", "pipe", "pipe"],
  });

  const debuggingPort = await waitForChromeLaunch(chrome);

  await waitForHttp(`http://localhost:${debuggingPort}/json/version`);

  return {
    chrome,
    debuggingPort,
    userDataDir,
  };
}

async function resolveRoutesToCheck(baseRef, changedFiles) {
  const allStaticRoutes = (await readPageFiles(path.resolve("src/app")))
    .map(routeFromPageFile)
    .filter((route) => route !== null);

  if (shouldSweepAllRoutes(baseRef, changedFiles)) {
    return Array.from(new Set(allStaticRoutes));
  }

  const changedRoutes = changedFiles
    .filter((filePath) => filePath === "src/app/page.tsx" || filePath.endsWith("/page.tsx"))
    .map(routeFromPageFile)
    .filter((route) => route !== null);

  return Array.from(new Set(changedRoutes));
}

async function auditRoute(route) {
  const routeUrl = `${LOCAL_BASE_URL}${route}`;
  if (await redirectsOffOrigin(routeUrl)) {
    console.log(`Lighthouse skipped ${route}; it redirects to external authentication.`);
    return;
  }

  const { chrome, debuggingPort, userDataDir } = await launchChrome();

  try {
    const result = await lighthouse(routeUrl, {
      logLevel: "error",
      onlyCategories: Object.keys(SCORE_THRESHOLDS),
      output: "json",
      port: debuggingPort,
    });

    const lhr = result?.lhr;

    if (!lhr) {
      throw new Error(`Lighthouse did not return an audit result for ${route}.`);
    }
    if (new URL(lhr.finalUrl).origin !== new URL(LOCAL_BASE_URL).origin) {
      console.log(`Lighthouse skipped ${route}; it redirects to external authentication.`);
      return;
    }

    await mkdir(LIGHTHOUSE_OUTPUT_DIR, { recursive: true });
    await writeFile(
      path.join(LIGHTHOUSE_OUTPUT_DIR, `${getRouteSlug(route)}.json`),
      getReportContent(result.report),
      "utf8",
    );

    const failures = Object.entries(SCORE_THRESHOLDS)
      .map(([category, minimumScore]) => {
        const score = lhr.categories[category]?.score ?? 0;
        return {
          category,
          minimumScore,
          score,
        };
      })
      .filter(({ minimumScore, score }) => score < minimumScore);

    if (failures.length > 0) {
      const message = failures
        .map(({ category, minimumScore, score }) => {
          return `${category} ${score.toFixed(2)} < ${minimumScore.toFixed(2)}`;
        })
        .join(", ");

      throw new Error(`Lighthouse thresholds failed for ${route}: ${message}`);
    }

    console.log(`Lighthouse passed for ${route}.`);
  } finally {
    chrome.kill("SIGTERM");
    await delay(1000);
    await rm(userDataDir, { force: true, recursive: true });
  }
}

async function redirectsOffOrigin(routeUrl) {
  const localOrigin = new URL(LOCAL_BASE_URL).origin;
  let currentUrl = routeUrl;
  for (let redirectCount = 0; redirectCount < 10; redirectCount += 1) {
    const response = await fetch(currentUrl, { redirect: "manual" });
    const location = response.headers.get("location");
    if (!location) return false;
    const nextUrl = new URL(location, currentUrl);
    if (nextUrl.origin !== localOrigin) return true;
    currentUrl = nextUrl.toString();
  }
  return false;
}

async function main() {
  const baseRef = getBaseRef();
  const changedFiles = Array.from(
    new Set([...getChangedFiles(baseRef), ...getWorkingTreeChangedFiles()]),
  );
  const routes = await resolveRoutesToCheck(baseRef, changedFiles);

  if (routes.length === 0) {
    console.log("No changed static routes detected. Skipping Lighthouse.");
    return;
  }

  console.log(`Running Lighthouse on: ${routes.join(", ")}`);

  await rm(LIGHTHOUSE_OUTPUT_DIR, { force: true, recursive: true });

  const server = spawn(
    "node",
    [
      path.join("node_modules", "next", "dist", "bin", "next"),
      "start",
      "--hostname",
      "localhost",
      "--port",
      LOCAL_BASE_URL.split(":").at(-1),
    ],
    {
      env: {
        ...process.env,
        PORT: LOCAL_BASE_URL.split(":").at(-1),
      },
      stdio: "inherit",
    },
  );

  try {
    await waitForHttp(`${LOCAL_BASE_URL}/`);

    for (const route of routes) {
      await auditRoute(route);
    }
  } finally {
    server.kill("SIGTERM");
    await delay(1000);
  }
}

await main();
