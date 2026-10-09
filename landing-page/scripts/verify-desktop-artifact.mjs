import { open, stat } from "node:fs/promises";
import path from "node:path";

const [artifactPath, expectedExtension] = process.argv.slice(2);

if (!artifactPath || !expectedExtension) {
  throw new Error("Usage: verify-desktop-artifact.mjs <artifact-path> <extension>");
}

const metadata = await stat(artifactPath);
if (!metadata.isFile()) {
  throw new Error(`${artifactPath} is not a file`);
}

if (metadata.size < 1024 * 1024) {
  throw new Error(`${artifactPath} is unexpectedly small: ${metadata.size} bytes`);
}

const extension = path.basename(artifactPath).split(".").pop();
if (extension !== expectedExtension) {
  throw new Error(`Expected .${expectedExtension} artifact, got ${artifactPath}`);
}

const header = await readBytes(artifactPath, 0, 64);
const validators = {
  AppImage: (bytes) =>
    isElf(bytes) &&
    bytes.readUInt16LE(18) === 0x3e &&
    bytes[8] === 0x41 &&
    bytes[9] === 0x49 &&
    [1, 2].includes(bytes[10]),
  deb: (bytes) => bytes.toString("utf8", 0, 8) === "!<arch>\n",
  dmg: (_bytes, trailer) => trailer.toString("ascii", 0, 4) === "koly",
  exe: (bytes, _trailer, peSignature) =>
    bytes[0] === 0x4d &&
    bytes[1] === 0x5a &&
    peSignature?.subarray(0, 4).toString("binary") === "PE\u0000\u0000",
  msi: (bytes) =>
    bytes[0] === 0xd0 &&
    bytes[1] === 0xcf &&
    bytes[2] === 0x11 &&
    bytes[3] === 0xe0 &&
    bytes[4] === 0xa1 &&
    bytes[5] === 0xb1 &&
    bytes[6] === 0x1a &&
    bytes[7] === 0xe1,
};

const validate = validators[expectedExtension];
if (!validate) {
  throw new Error(`No validator configured for .${expectedExtension}`);
}

const trailer =
  expectedExtension === "dmg"
    ? await readBytes(artifactPath, Math.max(0, metadata.size - 512), 512)
    : Buffer.alloc(0);
const peOffset = expectedExtension === "exe" ? header.readUInt32LE(0x3c) : null;
const peSignature = peOffset === null ? null : await readBytes(artifactPath, peOffset, 6);

if (!validate(header, trailer, peSignature)) {
  throw new Error(`${artifactPath} does not look like a .${expectedExtension} artifact`);
}

console.log(`verified ${artifactPath} (${metadata.size} bytes)`);

function isElf(bytes) {
  return bytes[0] === 0x7f && bytes[1] === 0x45 && bytes[2] === 0x4c && bytes[3] === 0x46;
}

async function readBytes(filePath, position, bytes) {
  const file = await open(filePath, "r");
  try {
    const buffer = Buffer.alloc(bytes);
    const { bytesRead } = await file.read(buffer, 0, bytes, position);
    return buffer.subarray(0, bytesRead);
  } finally {
    await file.close();
  }
}
