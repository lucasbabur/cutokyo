import { spawnSync } from "node:child_process";

// These are transitive findings in development-only CLIs and quality tools whose
// current owners do not yet expose compatible patched dependency ranges. Keep the
// baseline record-specific so a new advisory or affected package always fails CI.
const ACCEPTED_ADVISORIES = new Map([
  [1112659, "tar"],
  [1113300, "tar"],
  [1113375, "tar"],
  [1114200, "tar"],
  [1114302, "tar"],
  [1114680, "tar"],
  [1115540, "brace-expansion"],
  [1115541, "brace-expansion"],
  [1115543, "brace-expansion"],
  [1115549, "picomatch"],
  [1115551, "picomatch"],
  [1115552, "picomatch"],
  [1115554, "picomatch"],
  [1120311, "brace-expansion"],
  [1120782, "tar"],
  [1120793, "@babel/core"],
  [1121860, "js-yaml"],
  [1122892, "ws"],
]);

function readAuditReport(stdout) {
  const jsonStart = stdout.indexOf("{");
  if (jsonStart === -1) {
    throw new Error("bun audit did not return a JSON report");
  }

  return JSON.parse(stdout.slice(jsonStart));
}

const audit = spawnSync("bun", ["audit", "--json"], {
  encoding: "utf8",
  maxBuffer: 10 * 1024 * 1024,
});

if (audit.error) {
  throw audit.error;
}

if (audit.status !== 0 && audit.status !== 1) {
  process.stderr.write(audit.stderr);
  throw new Error(`bun audit failed with exit code ${audit.status ?? "unknown"}`);
}

const report = readAuditReport(audit.stdout);
const findings = Object.entries(report).flatMap(([packageName, advisories]) =>
  advisories.map((advisory) => ({ ...advisory, packageName })),
);
const unexpected = findings.filter(
  ({ id, packageName }) => ACCEPTED_ADVISORIES.get(id) !== packageName,
);

if (unexpected.length > 0) {
  for (const finding of unexpected) {
    console.error(
      `Unexpected ${finding.severity} advisory for ${finding.packageName}: ${finding.url}`,
    );
  }
  process.exit(1);
}

const resolvedCount = ACCEPTED_ADVISORIES.size - findings.length;
console.log(
  `Dependency audit passed: ${findings.length} accepted advisories remain, ${resolvedCount} resolved.`,
);
