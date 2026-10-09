import { createReadStream } from "node:fs";
import { writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";

const [artifactArgument] = process.argv.slice(2);
if (!artifactArgument) {
  throw new Error("Usage: write-desktop-checksum.mjs <artifact-path>");
}

const artifactPath = path.resolve(artifactArgument);
const hash = createHash("sha256");
for await (const chunk of createReadStream(artifactPath)) {
  hash.update(chunk);
}
const checksum = `${hash.digest("hex")}  ${path.basename(artifactPath)}\n`;
await writeFile(`${artifactPath}.sha256`, checksum, "utf8");
process.stdout.write(checksum);
