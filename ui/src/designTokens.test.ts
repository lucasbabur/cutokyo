import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const stylesheet = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), "styles.css"),
  "utf8",
);
const componentRules = stylesheet.slice(stylesheet.indexOf("@layer base {"));

describe("design tokens", () => {
  it("routes every font size through the type scale", () => {
    const raw = [...componentRules.matchAll(/font-size:\s*([^;]+);/g)]
      .map((match) => match[1] ?? "")
      .filter((value) => !value.includes("var(--font-size-"));
    expect(raw).toEqual([]);
  });

  it("gives checkbox-free capture management one full-width track", () => {
    const rule = /\.harness-choice--management\s*\{([^}]+)\}/.exec(
      stylesheet,
    )?.[1];
    expect(rule).toBeDefined();
    expect(rule).toMatch(/grid-template-columns:\s*minmax\(0,\s*1fr\)\s*;/);
    expect(rule).toMatch(/cursor:\s*default\s*;/);
    expect(rule).not.toMatch(/20px/);
    const choice = /\.harness-choice\s*\{([^}]+)\}/.exec(stylesheet)?.[1];
    expect(choice).toMatch(
      /grid-template-columns:\s*20px\s+minmax\(0,\s*1fr\)/,
    );
  });

  it("keeps the smallest type step legible", () => {
    const step = /--font-size-2xs:\s*(\d+)px/.exec(stylesheet)?.[1];
    expect(Number(step)).toBeGreaterThanOrEqual(10);
  });

  it("layers shared primitives above page component rules", () => {
    const order =
      /@layer ([\w, ]+);/.exec(stylesheet)?.[1]?.split(/,\s*/) ?? [];
    expect(order.indexOf("primitives")).toBeGreaterThan(
      order.indexOf("components"),
    );
    const components = stylesheet.slice(
      stylesheet.indexOf("@layer components {"),
      stylesheet.indexOf("@layer primitives {"),
    );
    expect(components).not.toMatch(/\.status-pill/);
  });
});
