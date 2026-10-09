import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { vi } from "vitest";

import { useOrganizationDashboard } from "./model";
import { OrganizationDashboardView } from "./view";
import { dashboardState, members, operation, policy, session } from "./view.test-fixtures";

vi.mock("./model", () => ({ useOrganizationDashboard: vi.fn() }));

describe("OrganizationDashboardView", () => {
  it("announces loading and error states", () => {
    vi.mocked(useOrganizationDashboard).mockReturnValue({
      ...dashboardState(),
      error: "Telemetry unavailable",
      loading: true,
    } as never);
    render(<OrganizationDashboardView section="employees" />);

    expect(screen.getByRole("alert")).toHaveTextContent("Telemetry unavailable");
    expect(screen.getByRole("status")).toHaveTextContent("Loading organization telemetry");
  });

  it("reserves the dashboard layout with a skeleton during its initial load", () => {
    vi.mocked(useOrganizationDashboard).mockReturnValue({
      ...dashboardState(),
      data: null,
      error: null,
      loading: true,
    } as never);

    render(<OrganizationDashboardView />);

    expect(
      screen.getByRole("status", { name: "Loading organization telemetry" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("dashboard-skeleton")).toBeInTheDocument();
    expect(screen.queryByText("$0.00")).not.toBeInTheDocument();
  });

  it("shows employee emails instead of internal user IDs in the Users breakdown", () => {
    const state = dashboardState();
    if (!state.data) throw new Error("Dashboard fixture must include data");
    state.data.members = members;
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);

    render(<OrganizationDashboardView />);

    const usersCard = screen.getByRole("article", { name: "Users" });
    expect(within(usersCard).getByText("ada@example.com")).toBeInTheDocument();
    expect(within(usersCard).queryByText("User Ada")).not.toBeInTheDocument();
  });

  it("supports user and group drilldowns plus real group membership mutations", async () => {
    const state = dashboardState();
    if (!state.data) throw new Error("Dashboard fixture must include data");
    state.data.members = members;
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);
    render(<OrganizationDashboardView section="employees" />);

    const scope = screen.getByRole("combobox", { name: "Telemetry scope" });
    expect(scope).toHaveValue("all");
    expect(screen.getByRole("option", { name: "Group · Platform" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "User · ada@example.com" })).toBeInTheDocument();

    fireEvent.change(scope, { target: { value: "group:grp_platform" } });
    expect(state.setTelemetryScope).toHaveBeenCalledWith({
      id: "grp_platform",
      kind: "group",
      label: "Platform",
    });

    fireEvent.change(scope, { target: { value: "user:user_ada" } });
    expect(state.setTelemetryScope).toHaveBeenCalledWith({
      id: "user_ada",
      kind: "user",
      label: "ada@example.com",
    });

    fireEvent.change(screen.getByLabelText("Group name"), { target: { value: "Research" } });
    fireEvent.click(screen.getByRole("button", { name: /Ada Lovelace.*ada@example.com.*Add/i }));
    fireEvent.click(screen.getByRole("button", { name: /Grace Hopper.*grace@example.com.*Add/i }));
    fireEvent.click(screen.getByRole("button", { name: "Create group" }));

    await waitFor(() =>
      expect(state.saveGroup).toHaveBeenCalledWith({
        memberIds: ["user_ada", "user_grace"],
        name: "Research",
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Delete Platform" }));
    expect(state.deleteGroup).toHaveBeenCalledWith("grp_platform");
  });

  it("shows named employee usage and redacted session content on their routes", () => {
    const state = dashboardState();
    if (!state.data) throw new Error("Dashboard fixture must include data");
    state.data.members = members.slice(0, 1);
    state.data.sessions = [session];
    state.permissions.push("sessions:read");
    state.selectedSessionId = session.id;
    state.sessionDetail = {
      ...session,
      operationItems: [operation],
      operationItemsTruncated: false,
      messages: [
        {
          callId: null,
          content: "Contact [REDACTED:email] about the incident",
          kind: "message",
          name: null,
          role: "user",
          timestamp: "2026-07-10T12:00:00Z",
          traceId: "trace-ada",
          truncated: false,
        },
      ],
    };
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);

    const view = render(<OrganizationDashboardView section="employees" />);

    expect(screen.getByRole("heading", { level: 2, name: "Employee usage" })).toBeInTheDocument();
    expect(screen.getAllByText("Ada Lovelace").length).toBeGreaterThan(0);
    expect(screen.getByText("Developer")).toBeInTheDocument();

    view.rerender(<OrganizationDashboardView section="sessions" />);

    expect(screen.getByText("Contact [REDACTED:email] about the incident")).toBeInTheDocument();
    expect(screen.getByText("Session operations")).toBeInTheDocument();
    expect(screen.getAllByText("request_ada").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: /Cutokyo implementation/i }));
    expect(state.setSelectedSessionId).toHaveBeenCalledWith("session-ada");
  });

  it("filters sessions by employee email and search text", () => {
    const state = dashboardState();
    if (!state.data) throw new Error("Dashboard fixture must include data");
    state.data.members = members;
    state.data.sessions = [session];
    state.permissions.push("sessions:read");
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);

    render(<OrganizationDashboardView section="sessions" />);

    expect(screen.getByText(/ada@example.com · Codex CLI/i)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Filter sessions by employee"), {
      target: { value: "user_grace" },
    });
    expect(screen.getByText("No employee sessions match these filters.")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Filter sessions by employee"), {
      target: { value: "all" },
    });
    fireEvent.change(screen.getByLabelText("Search employee sessions"), {
      target: { value: "ada@example.com" },
    });
    expect(screen.getByRole("button", { name: /Cutokyo implementation/i })).toBeInTheDocument();
  });

  it("investigates exact operations and browses older cursor pages", () => {
    const state = dashboardState();
    if (!state.data) throw new Error("Dashboard fixture must include data");
    state.data.members = members;
    state.permissions.push("telemetry:read");
    state.operationCorrelation = "request_ada";
    state.operationPage = { nextCursor: 7, operations: [operation] };
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);

    render(<OrganizationDashboardView section="activity" />);

    const operationSummary = screen.getAllByText("request_ada").at(0);
    if (!operationSummary) throw new Error("Operation summary must be rendered");
    fireEvent.click(operationSummary);
    expect(screen.getByText("trace_ada")).toBeInTheDocument();
    expect(screen.getByText("response_ada")).toBeInTheDocument();
    expect(screen.getByText("error · provider_timeout")).toBeInTheDocument();
    expect(screen.getByText("/v1/responses · http")).toBeInTheDocument();
    expect(screen.getByText("430 / 12 ms / 418 ms / 200 ms")).toBeInTheDocument();
    expect(screen.getByText("100 / 180 / 40")).toBeInTheDocument();
    expect(screen.getByText("80 / 9 / 12 / 241")).toBeInTheDocument();
    expect(screen.getByText("$0.00099 · 2026-07-09")).toBeInTheDocument();
    expect(screen.getByText(/relevance-v1/)).toBeInTheDocument();
    expect(screen.getByText(/request-audit/)).toBeInTheDocument();
    expect(screen.getByText("Policy-redacted preview available")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Find exact" }));
    expect(state.loadOperations).toHaveBeenCalledWith("request_ada");
    fireEvent.click(screen.getByRole("button", { name: "Load older operations" }));
    expect(state.loadOperations).toHaveBeenLastCalledWith("request_ada", 7);
  });

  it("lets administrators manage enterprise redaction and compression defaults", () => {
    const state = dashboardState();
    if (!state.data) throw new Error("Dashboard fixture must include data");
    state.data.policy = policy;
    state.permissions.push("policies:read", "policies:write");
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);

    render(<OrganizationDashboardView section="enterprise" />);

    fireEvent.click(screen.getByRole("button", { name: "Organization token compression default" }));
    expect(state.setPolicyCapability).toHaveBeenCalledWith("optimization", false);

    fireEvent.click(screen.getByRole("button", { name: "Email addresses" }));
    expect(state.setPolicyCapability).toHaveBeenCalledWith("email-redaction", false);
    expect(screen.getByText("Managed by administrators")).toBeInTheDocument();
  });

  it("keeps suspended installations readable while disabling enterprise writes", () => {
    const state = dashboardState();
    if (!state.data) throw new Error("Dashboard fixture must include data");
    state.data.policy = policy;
    state.data.license = {
      ...state.data.license,
      detail: "Commercial writes are unavailable; reads and local safety remain available.",
      mode: "enforced",
      readOnly: true,
      state: "suspended",
    };
    state.permissions.push("policies:read", "policies:write");
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);

    render(<OrganizationDashboardView section="enterprise" />);

    expect(screen.getByText("This installation is read-only")).toBeInTheDocument();
    expect(
      screen.getByText("Reads, export, backups, and local safety remain available."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Export organization data" }));
    expect(state.exportOrganizationData).toHaveBeenCalledOnce();
    expect(
      screen.getByRole("button", { name: "Organization token compression default" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "Read-only" })).toBeDisabled();
  });

  it("installs and controls an organization log plugin without leaking extra fields", async () => {
    const state = dashboardState();
    if (!state.data) throw new Error("Dashboard fixture must include data");
    state.permissions.push("policies:read", "policies:write");
    state.data.plugins = [
      {
        catalogId: "webhook-logs",
        createdAt: "2026-07-10T12:00:00Z",
        deliveredEvents: 4,
        enabled: true,
        endpoint: "https://logs.example.com/cutokyo",
        hasSecret: true,
        id: "plg_security",
        lastAttemptAt: null,
        lastError: null,
        lastSuccessAt: "2026-07-10T12:01:00Z",
        name: "Security stream",
        updatedAt: "2026-07-10T12:00:00Z",
      },
    ];
    vi.mocked(useOrganizationDashboard).mockReturnValue(state as never);

    render(<OrganizationDashboardView section="enterprise" />);
    fireEvent.change(screen.getByLabelText("Installation name"), {
      target: { value: "Audit collector" },
    });
    fireEvent.change(screen.getByLabelText("HTTPS endpoint"), {
      target: { value: "https://audit.example.com/events" },
    });
    fireEvent.change(screen.getByLabelText(/Bearer token/i), {
      target: { value: "destination-secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Install plugin" }));

    await waitFor(() =>
      expect(state.saveManagedPlugin).toHaveBeenCalledWith({
        enabled: true,
        endpoint: "https://audit.example.com/events",
        name: "Audit collector",
        secret: "destination-secret",
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(state.saveManagedPlugin).toHaveBeenLastCalledWith({
      enabled: false,
      endpoint: "https://logs.example.com/cutokyo",
      id: "plg_security",
      name: "Security stream",
    });
  });
});
