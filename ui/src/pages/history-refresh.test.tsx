import { render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { CommandProvider } from "../commands/context.js";
import { AnnouncementProvider } from "../components/Announcer.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { DashboardPage } from "./DashboardPage.js";
import { SessionsPage } from "./SessionsPage.js";

describe("background history imports", () => {
  it("refresh the sessions list and overview when new history arrives", async () => {
    const client = createBrowserFixtureClient("populated-dashboard");
    const searchSessions = vi.fn(client.searchSessions);
    const getDashboard = vi.fn(client.getDashboard);
    render(
      <CommandProvider client={{ ...client, searchSessions, getDashboard }}>
        <AnnouncementProvider>
          <DashboardPage />
          <SessionsPage sessionId={null} />
        </AnnouncementProvider>
      </CommandProvider>,
    );
    await waitFor(() => expect(searchSessions).toHaveBeenCalled());
    await waitFor(() => expect(getDashboard).toHaveBeenCalled());
    const searches = searchSessions.mock.calls.length;
    const dashboards = getDashboard.mock.calls.length;
    client.emitHistoryImported();
    await waitFor(() =>
      expect(searchSessions.mock.calls.length).toBeGreaterThan(searches),
    );
    await waitFor(() =>
      expect(getDashboard.mock.calls.length).toBeGreaterThan(dashboards),
    );
  });
});
