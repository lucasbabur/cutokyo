import { describe, expect, it } from "vitest";

import { publicHttpsUrlSchema, publicHttpUrlSchema } from "./env";

describe("public URL schemas", () => {
  it.each(["http://localhost:3000", "https://cutokyo.example/app"])(
    "accepts an HTTP(S) application URL: %s",
    (url) => {
      expect(publicHttpUrlSchema.safeParse(url).success).toBe(true);
    },
  );

  it.each(["javascript:alert(1)", "data:text/html,unsafe", "file:///tmp/cutokyo"])(
    "rejects a non-HTTP application URL: %s",
    (url) => {
      expect(publicHttpUrlSchema.safeParse(url).success).toBe(false);
    },
  );

  it("requires release downloads to use HTTPS", () => {
    expect(publicHttpsUrlSchema.safeParse("https://releases.example/cutokyo").success).toBe(true);
    expect(publicHttpsUrlSchema.safeParse("http://releases.example/cutokyo").success).toBe(false);
  });
});
