import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";

function readStagedFiles() {
  const output = execFileSync(
    "git",
    ["diff", "--cached", "--name-only", "--relative", "--diff-filter=ACMR"],
    {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "inherit"],
    },
  );

  return output
    .split("\n")
    .map((filePath) => filePath.trim())
    .filter(Boolean)
    .filter((filePath) => existsSync(filePath));
}

const stagedFiles = readStagedFiles();

if (stagedFiles.length === 0) {
  console.log("No staged files to scan for secrets.");
  process.exit(0);
}

execFileSync("bunx", ["secretlint", ...stagedFiles], {
  stdio: "inherit",
});
