import { describe, expect, it } from "vitest";

import { trackInputModality } from "./inputModality.js";

describe("input modality", () => {
  it("hides focus rings after a pointer press and restores them on a key press", () => {
    const root = document.documentElement;
    trackInputModality(root);
    expect(root.dataset["input"]).toBe("keyboard");
    globalThis.dispatchEvent(new Event("pointerdown"));
    expect(root.dataset["input"]).toBe("pointer");
    globalThis.dispatchEvent(
      new KeyboardEvent("keydown", { key: "c", ctrlKey: true }),
    );
    expect(root.dataset["input"]).toBe("pointer");
    globalThis.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
    expect(root.dataset["input"]).toBe("keyboard");
  });
});
