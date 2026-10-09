import { describe, expect, it } from "vitest";

import { applySettingsPatch } from "./contracts.js";
import { SETTINGS_PATCH_SCHEMA_ID } from "./generated/settings.js";

describe("settings contract", () => {
  it("preserves controls omitted by a patch", () => {
    const current = {
      proxy_enabled: false,
      search_mcp_enabled: true,
      retention_days: null,
    };
    const updated = applySettingsPatch(current, { proxy_enabled: true });
    expect(updated.proxy_enabled).toBe(true);
    expect(updated.search_mcp_enabled).toBe(true);
  });

  it("keeps the stable schema identity visible", () => {
    expect(SETTINGS_PATCH_SCHEMA_ID).toBe(
      "https://schemas.cutokyo.dev/settings/patch/v1",
    );
  });
});
