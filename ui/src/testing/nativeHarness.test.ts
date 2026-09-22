import { describe, expect, it } from "vitest";

import { assertNativeGeometry } from "../../tests/tauri/geometry.js";

describe("native screenshot geometry", () => {
  it("accepts only the exact native window and viewport advertised by the screenshot", () => {
    const exact = { width: 1280, height: 800 };
    expect(() => assertNativeGeometry(exact, exact)).not.toThrow();
    for (const wrong of [
      { width: 900, height: 700 },
      { width: 1536, height: 960 },
      { width: 1280, height: 799 },
      { width: 1279, height: 800 },
    ]) {
      expect(() => assertNativeGeometry(wrong, exact)).toThrow();
      expect(() => assertNativeGeometry(exact, wrong)).toThrow();
    }
  });
});
