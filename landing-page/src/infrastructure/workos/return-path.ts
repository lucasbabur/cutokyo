const ADMIN_RETURN_PATHS = new Set([
  "/admin",
  "/admin/account",
  "/admin/activity",
  "/admin/employees",
  "/admin/enterprise",
  "/admin/governance",
  "/admin/sessions",
]);

export function getAdminReturnPath(value: string | null | undefined): string {
  return value && ADMIN_RETURN_PATHS.has(value) ? value : "/admin";
}
