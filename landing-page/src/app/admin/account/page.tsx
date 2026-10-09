import { withAuth } from "@workos-inc/authkit-nextjs";
import { redirect } from "next/navigation";

import { signOutAction } from "@/app/auth/actions";
import { OrganizationAccountView } from "@/features/organization-dashboard";

export default async function AccountPage() {
  const auth = await withAuth();
  if (!auth.user) redirect("/sign-in?returnTo=%2Fadmin%2Faccount");
  const { organizationId, permissions, role, roles, user } = auth;

  return (
    <OrganizationAccountView
      organizationId={organizationId ?? "No active organization"}
      permissions={permissions ?? []}
      role={role ?? roles?.at(0) ?? "Member"}
      signOutAction={signOutAction}
      user={user}
    />
  );
}
