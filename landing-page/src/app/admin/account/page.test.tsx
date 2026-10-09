import { render, screen } from "@testing-library/react";
import { vi } from "vitest";

import AccountPage from "./page";

const { redirect, withAuth } = vi.hoisted(() => ({
  redirect: vi.fn((destination: string) => {
    throw new Error(`redirect:${destination}`);
  }),
  withAuth: vi.fn(),
}));

vi.mock("@workos-inc/authkit-nextjs", () => ({ withAuth }));
vi.mock("next/navigation", () => ({ redirect }));
vi.mock("@/app/auth/actions", () => ({ signOutAction: vi.fn() }));
vi.mock("@/features/organization-dashboard", () => ({
  OrganizationAccountView: ({ user }: { user: { firstName: string; lastName: string } }) => (
    <h1>{`${user.firstName} ${user.lastName}`}</h1>
  ),
}));

describe("AccountPage", () => {
  it("redirects signed-out requests through the route handler", async () => {
    withAuth.mockResolvedValue({ user: null });

    await expect(AccountPage()).rejects.toThrow("redirect:/sign-in?returnTo=%2Fadmin%2Faccount");
    expect(withAuth).toHaveBeenCalledWith();
    expect(redirect).toHaveBeenCalledWith("/sign-in?returnTo=%2Fadmin%2Faccount");
  });

  it("renders the authenticated account without mutating cookies", async () => {
    withAuth.mockResolvedValue({
      organizationId: "org_cutokyo",
      permissions: ["policies:read"],
      role: "admin",
      roles: ["admin"],
      user: {
        email: "lucas@example.com",
        firstName: "Lucas",
        id: "user_lucas",
        lastName: "Bolado",
      },
    });

    render(await AccountPage());

    expect(screen.getByRole("heading", { name: "Lucas Bolado" })).toBeInTheDocument();
    expect(withAuth).toHaveBeenCalledWith();
  });
});
