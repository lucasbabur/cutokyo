import { OrganizationDashboardView } from "@/features/organization-dashboard";
import { isWorkosConfigured } from "@/infrastructure/workos/config";

export default function EmployeesPage() {
  return <OrganizationDashboardView section="employees" workosConfigured={isWorkosConfigured()} />;
}
