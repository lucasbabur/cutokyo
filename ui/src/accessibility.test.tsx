import axe from "axe-core";
import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";

import { CommandProvider } from "./commands/context.js";
import { AnnouncementProvider } from "./components/Announcer.js";
import type { CommandClient } from "./contracts.js";
import { createBrowserFixtureClient } from "./fixtures/browserAdapter.js";
import { DashboardPage } from "./pages/DashboardPage.js";
import { DataControlsPage } from "./pages/DataControlsPage.js";
import { HealthPage } from "./pages/HealthPage.js";
import { InventoryPage } from "./pages/InventoryPage.js";
import { OnboardingPage } from "./pages/OnboardingPage.js";
import { SessionsPage } from "./pages/SessionsPage.js";
import { SettingsPage } from "./pages/SettingsPage.js";

function renderRoute(client: CommandClient, node: ReactNode) {
  return render(
    <CommandProvider client={client}>
      <AnnouncementProvider>{node}</AnnouncementProvider>
    </CommandProvider>,
  );
}

const routes: readonly {
  name: string;
  heading: string;
  scenario: string;
  node: ReactNode;
}[] = [
  {
    name: "onboarding",
    heading: "Set up Cutokyo",
    scenario: "onboarding-empty",
    node: <OnboardingPage mode="onboarding" onComplete={() => undefined} />,
  },
  {
    name: "dashboard",
    heading: "Overview",
    scenario: "populated-dashboard",
    node: <DashboardPage />,
  },
  {
    name: "empty history",
    heading: "Sessions",
    scenario: "empty-history",
    node: <SessionsPage sessionId={null} />,
  },
  {
    name: "inventory",
    heading: "Agent tools",
    scenario: "mcp-plugin-inventory",
    node: <InventoryPage />,
  },
  {
    name: "degraded health",
    heading: "Health",
    scenario: "health-degraded",
    node: <HealthPage />,
  },
  {
    name: "data controls",
    heading: "Data controls",
    scenario: "retention-delete",
    node: <DataControlsPage />,
  },
  {
    name: "settings",
    heading: "Settings",
    scenario: "visual-keyboard",
    node: <SettingsPage />,
  },
];

describe("route accessibility", () => {
  it.each(routes)(
    "has no detectable violations on $name",
    async ({ heading, scenario, node }) => {
      const { container } = renderRoute(
        createBrowserFixtureClient(scenario),
        node,
      );
      await screen.findByRole("heading", { name: heading });
      const scan = await axe.run(container);
      expect(scan.violations).toEqual([]);
    },
  );
});
