import { describe, expect, it } from "vitest";

import { resolveWidgetScope } from "./widget-authorization";

describe("resolveWidgetScope", () => {
  it.each([
    ["users", "widgets:users-table:manage"],
    ["sso", "widgets:sso:manage"],
    ["domains", "widgets:domain-verification:manage"],
    ["dsync", "widgets:dsync:manage"],
    ["audit", "widgets:audit-log-streaming:manage"],
  ])("maps %s to its WorkOS widget permission", (key, scope) => {
    expect(resolveWidgetScope(key)).toBe(scope);
  });

  it("rejects unknown widget keys", () => {
    expect(resolveWidgetScope("billing")).toBeNull();
  });
});
