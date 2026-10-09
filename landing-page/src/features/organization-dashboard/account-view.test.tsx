import { render, screen } from "@testing-library/react";
import { vi } from "vitest";

import { OrganizationAccountView } from "./account-view";

describe("OrganizationAccountView", () => {
  it("shows identity, organization access, permissions, and a POST action logout control", () => {
    const signOutAction = vi.fn().mockResolvedValue(undefined);
    render(
      <OrganizationAccountView
        organizationId="org_cutokyo"
        permissions={["sessions:read", "audit:read"]}
        role="cutokyo-admin"
        signOutAction={signOutAction}
        user={{
          createdAt: "2026-07-01T12:00:00Z",
          email: "lucas@example.com",
          emailVerified: true,
          firstName: "Lucas",
          id: "user_lucas",
          lastName: "Babur",
          lastSignInAt: "2026-07-13T12:00:00Z",
          name: "Lucas Babur",
        }}
      />,
    );

    expect(screen.getByRole("heading", { name: "Your account" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Lucas Babur" })).toBeInTheDocument();
    expect(screen.getByText("Cutokyo Admin")).toBeInTheDocument();
    expect(screen.getByText("sessions:read")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Sync Google profile photo" })).toHaveAttribute(
      "href",
      "/sign-in?returnTo=%2Fadmin%2Faccount&reauth=google",
    );
    expect(screen.getByRole("button", { name: "Sign out" })).toHaveAttribute("type", "submit");
  });

  it("renders the WorkOS profile picture when the provider supplies one", () => {
    render(
      <OrganizationAccountView
        organizationId="org_cutokyo"
        permissions={[]}
        role="member"
        signOutAction={vi.fn().mockResolvedValue(undefined)}
        user={{
          createdAt: "2026-07-01T12:00:00Z",
          email: "lucas@example.com",
          emailVerified: true,
          firstName: "Lucas",
          id: "user_lucas",
          lastName: "Babur",
          lastSignInAt: "2026-07-13T12:00:00Z",
          name: "Lucas Babur",
          profilePictureUrl: "https://images.example.com/lucas.png",
        }}
      />,
    );

    expect(screen.getByRole("img", { name: "Lucas Babur profile" })).toHaveAttribute(
      "src",
      "https://images.example.com/lucas.png",
    );
    expect(
      screen.queryByRole("link", { name: "Sync Google profile photo" }),
    ).not.toBeInTheDocument();
  });
});
