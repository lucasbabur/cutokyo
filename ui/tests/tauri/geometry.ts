import { strict as assert } from "node:assert";

export function assertNativeGeometry(
  windowSize: { width: number; height: number },
  viewport: { width: number; height: number },
): void {
  assert.deepEqual(windowSize, { width: 1280, height: 800 });
  assert.deepEqual(viewport, { width: 1280, height: 800 });
}
