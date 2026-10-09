import { OrganizationDashboardView } from "@/features/organization-dashboard";
import { isWorkosConfigured } from "@/infrastructure/workos/config";

export default function AdminPage() {
  return <OrganizationDashboardView workosConfigured={isWorkosConfigured()} />;
}
