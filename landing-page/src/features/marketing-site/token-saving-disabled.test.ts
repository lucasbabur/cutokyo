import { describe, expect, it, vi } from "vitest";

import { activeMarketingCopies } from "./i18n";

vi.mock("@/shared/config/features", () => ({ tokenSavingEnabled: false }));

describe("landing page copy without token saving", () => {
  it("makes no token reduction or cost savings claim in English", () => {
    const copy = activeMarketingCopies.en;

    expect(copy.hero.title).not.toMatch(/save/i);
    expect(copy.hero.kicker).not.toMatch(/compress/i);
    expect(copy.hero.body).not.toMatch(/cheaper|wasted tokens/i);
    expect(copy.footer.description).not.toMatch(/70%|~50%/);
    expect(copy.footer.copyright).not.toMatch(/cheaper/i);
    expect(copy.steps.some((step) => step.percent !== undefined)).toBe(false);
    expect(copy.steps.some((step) => step.icon === "savings")).toBe(false);
    expect(copy.cards.some(([, title]) => /compression/i.test(title))).toBe(false);
  });

  it("makes no token reduction or cost savings claim in Portuguese", () => {
    const copy = activeMarketingCopies["pt-BR"];

    expect(copy.hero.kicker).not.toMatch(/compressor/i);
    expect(copy.footer.description).not.toMatch(/70%|~50%/);
    expect(copy.steps.some((step) => step.percent !== undefined)).toBe(false);
    expect(copy.cards.some(([, title]) => /compress/i.test(title))).toBe(false);
  });

  it("keeps the shared page structure intact", () => {
    for (const locale of ["en", "pt-BR"] as const) {
      const copy = activeMarketingCopies[locale];
      expect(copy.steps).toHaveLength(3);
      expect(copy.cards.length).toBeGreaterThan(0);
      expect(copy.download.title).toBeTruthy();
      expect(copy.controlPlane.title).toBeTruthy();
    }
  });
});
