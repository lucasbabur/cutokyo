import { OrganizationDashboardView } from "@/features/organization-dashboard";
import { isWorkosConfigured } from "@/infrastructure/workos/config";

export default function EnterpriseAdminPage() {
  return <OrganizationDashboardView section="enterprise" workosConfigured={isWorkosConfigured()} />;
}
