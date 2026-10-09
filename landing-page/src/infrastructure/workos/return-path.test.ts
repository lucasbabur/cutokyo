import { getAdminReturnPath } from "./return-path";

describe("getAdminReturnPath", () => {
  it.each([
    "/admin",
    "/admin/account",
    "/admin/activity",
    "/admin/employees",
    "/admin/enterprise",
    "/admin/governance",
    "/admin/sessions",
  ])("keeps the known admin destination %s", (path) => {
    expect(getAdminReturnPath(path)).toBe(path);
  });

  it.each([undefined, null, "", "/", "//evil.example", "https://evil.example/admin"])(
    "falls back to the admin home for %s",
    (path) => {
      expect(getAdminReturnPath(path)).toBe("/admin");
    },
  );
});
