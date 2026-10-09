import { spawn } from "node:child_process";
import process from "node:process";

const STATIC_CHECKS = [
  { label: "format", command: ["bun", "run", "format"] },
  { label: "lint", command: ["bun", "run", "lint"] },
  { label: "typecheck", command: ["bun", "run", "typecheck"] },
  { label: "depcruise", command: ["bun", "run", "depcruise"] },
  { label: "knip", command: ["bun", "run", "knip"] },
];

const FAST_CHECKS = [
  { label: "lint", command: ["bun", "run", "lint"] },
  { label: "typecheck", command: ["bun", "run", "typecheck"] },
  { label: "depcruise", command: ["bun", "run", "depcruise"] },
];

const TEST_CHECKS = [{ label: "test", command: ["bun", "run", "test"] }];

const PUSH_CHECKS = [
  { label: "build", command: ["bun", "run", "build"] },
  { label: "lighthouse:changed", command: ["bun", "run", "lighthouse:changed"] },
];

function formatSeconds(durationMs) {
  return `${(durationMs / 1000).toFixed(1)}s`;
}

function runCheck(check) {
  const startedAt = Date.now();

  return new Promise((resolve) => {
    const child = spawn(check.command[0], check.command.slice(1), {
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", (error) => {
      resolve({
        ...check,
        code: null,
        durationMs: Date.now() - startedAt,
        stderr: error instanceof Error ? error.message : String(error),
        stdout,
      });
    });
    child.on("close", (code) => {
      resolve({
        ...check,
        code,
        durationMs: Date.now() - startedAt,
        stderr,
        stdout,
      });
    });
  });
}

function printResult(result) {
  console.log(`\n## ${result.label}`);
  console.log(`$ ${result.command.join(" ")}`);
  console.log(`exit: ${result.code ?? "spawn-error"} (${formatSeconds(result.durationMs)})`);

  const output = [result.stdout.trim(), result.stderr.trim()].filter(Boolean).join("\n");
  if (output.length > 0) {
    console.log(output);
  }
}

async function runParallel(checks) {
  console.log(
    `Running ${checks.length} checks in parallel: ${checks.map((check) => check.label).join(", ")}`,
  );
  const results = await Promise.all(checks.map((check) => runCheck(check)));

  for (const result of results) {
    printResult(result);
  }

  const failed = results.filter((result) => result.code !== 0);
  if (failed.length > 0) {
    throw new Error(`Failed checks: ${failed.map((result) => result.label).join(", ")}`);
  }
}

async function runSerial(checks) {
  for (const check of checks) {
    const result = await runCheck(check);
    printResult(result);

    if (result.code !== 0) {
      throw new Error(`Failed check: ${result.label}`);
    }
  }
}

async function main() {
  const mode = process.argv[2] ?? "validate";

  if (mode === "check:fast") {
    await runParallel(FAST_CHECKS);
    return;
  }

  if (mode === "validate") {
    await runParallel(STATIC_CHECKS);
    await runSerial(TEST_CHECKS);
    return;
  }

  if (mode === "validate:push") {
    await runParallel(STATIC_CHECKS);
    await runSerial(TEST_CHECKS);
    await runSerial(PUSH_CHECKS);
    return;
  }

  throw new Error(`Unknown check mode: ${mode}`);
}

try {
  await main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
