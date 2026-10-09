import { OrganizationDashboardView } from "@/features/organization-dashboard";
import { isWorkosConfigured } from "@/infrastructure/workos/config";

export default function ActivityPage() {
  return <OrganizationDashboardView section="activity" workosConfigured={isWorkosConfigured()} />;
}
