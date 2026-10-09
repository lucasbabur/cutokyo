import { OrganizationDashboardView } from "@/features/organization-dashboard";
import { isWorkosConfigured } from "@/infrastructure/workos/config";

export default function GovernancePage() {
  return <OrganizationDashboardView section="governance" workosConfigured={isWorkosConfigured()} />;
}
