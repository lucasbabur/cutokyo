import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute } from "node:path";

function nativeAssets(): string {
  const output = process.env.CUTOKYO_NATIVE_ASSET_DIR;
  if (
    !output ||
    !isAbsolute(output) ||
    basename(output) !== "assets" ||
    !basename(dirname(output)).startsWith("cutokyo-native-test-wdio-") ||
    realpathSync(dirname(output)) !== dirname(output) ||
    dirname(dirname(output)) !== realpathSync(tmpdir())
  ) {
    throw new Error(
      "Native assets require the isolated package runner; ui/dist is production-only.",
    );
  }
  return output;
}

export default defineConfig(({ mode }) => ({
  plugins: [react()],
  clearScreen: false,
  envPrefix: ["VITE_"],
  server: {
    host: "127.0.0.1",
    port: 4173,
    strictPort: true,
  },
  preview: {
    host: "127.0.0.1",
    port: 4173,
    strictPort: true,
  },
  build: {
    target: "es2022",
    sourcemap: false,
    outDir: mode === "native-e2e" ? nativeAssets() : "dist",
    emptyOutDir: true,
  },
}));
