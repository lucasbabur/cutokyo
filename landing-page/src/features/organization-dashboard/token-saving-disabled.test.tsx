import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useOrganizationDashboard } from "./model";
import { OrganizationDashboardView } from "./view";
import { dashboardState, policy } from "./view.test-fixtures";

vi.mock("./model", () => ({ useOrganizationDashboard: vi.fn() }));
vi.mock("@/shared/config/features", () => ({ tokenSavingEnabled: false }));

describe("organization dashboard without token saving", () => {
  it("omits the tokens removed metric from the overview", () => {
    const state = dashboardState();
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);

    render(<OrganizationDashboardView section="overview" />);

    expect(screen.queryByText("Tokens removed")).not.toBeInTheDocument();
    // The rest of the metrics row still renders.
    expect(screen.getByText("Provider tokens")).toBeInTheDocument();
    expect(screen.getByText("Estimated cost")).toBeInTheDocument();
  });

  it("omits the organization token compression default", () => {
    const state = dashboardState();
    if (!state.data) throw new Error("Dashboard fixture must include data");
    state.data.policy = policy;
    state.permissions.push("policies:read", "policies:write");
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);

    render(<OrganizationDashboardView section="enterprise" />);

    expect(
      screen.queryByRole("button", { name: "Organization token compression default" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Token compression")).not.toBeInTheDocument();
    expect(screen.queryByText(/compression/i)).not.toBeInTheDocument();
    // Redaction defaults are unaffected.
    expect(screen.getByRole("button", { name: "Email addresses" })).toBeInTheDocument();
  });
});
