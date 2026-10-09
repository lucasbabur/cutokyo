import { OrganizationDashboardView } from "@/features/organization-dashboard";
import { isWorkosConfigured } from "@/infrastructure/workos/config";

export default function SessionsPage() {
  return <OrganizationDashboardView section="sessions" workosConfigured={isWorkosConfigured()} />;
}
