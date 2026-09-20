import { readdir, readFile, stat } from "node:fs/promises";
import { resolve } from "node:path";

const sentinels = [
  "jev_case",
  "__CUTOKYO_FIXTURE_AUDIT__",
  "Unknown isolated browser fixture",
  "JEV exact resume needle 73A9",
  "delete-me-73A9",
  "fixture-parser-1",
] as const;

async function filesBelow(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const path = resolve(directory, entry.name);
      return entry.isDirectory() ? filesBelow(path) : [path];
    }),
  );
  return files.flat();
}

const dist = resolve(import.meta.dirname, "../dist");
let distStats;
try {
  distStats = await stat(dist);
} catch {
  throw new Error(
    "Production dist is missing. Run the Vite build before this check.",
  );
}
if (!distStats.isDirectory())
  throw new Error("Production dist is not a directory.");

const leaks: string[] = [];
for (const file of await filesBelow(dist)) {
  const content = await readFile(file, "utf8").catch(() => null);
  if (content === null) continue;
  for (const sentinel of sentinels) {
    if (content.includes(sentinel)) leaks.push(`${sentinel} in ${file}`);
  }
}

if (leaks.length > 0) {
  throw new Error(
    `Browser fixture code leaked into production:\n${leaks.join("\n")}`,
  );
}

console.log(
  "Production bundle contains no browser fixture selector, audit hook, or sentinel data.",
);
