import type { WidgetSessionTokenScopes } from "@workos-inc/node";

const widgetScopes = {
  audit: "widgets:audit-log-streaming:manage",
  domains: "widgets:domain-verification:manage",
  dsync: "widgets:dsync:manage",
  sso: "widgets:sso:manage",
  users: "widgets:users-table:manage",
} as const satisfies Record<string, WidgetSessionTokenScopes>;

type WidgetScopeKey = keyof typeof widgetScopes;

export function resolveWidgetScope(value: string): WidgetSessionTokenScopes | null {
  return value in widgetScopes ? widgetScopes[value as WidgetScopeKey] : null;
}
