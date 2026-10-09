import { useEffect, useState } from "react";

import { useCommands, useCommandResource } from "./commands/context.js";
import { AppShell } from "./components/AppShell.js";
import { ErrorState, LoadingState } from "./components/Primitives.js";
import { DashboardPage } from "./pages/DashboardPage.js";
import { DataControlsPage } from "./pages/DataControlsPage.js";
import { HealthPage } from "./pages/HealthPage.js";
import { InventoryPage } from "./pages/InventoryPage.js";
import { OnboardingPage } from "./pages/OnboardingPage.js";
import { SessionsPage } from "./pages/SessionsPage.js";
import { SettingsPage } from "./pages/SettingsPage.js";
import { navigate, useAppLocation } from "./router.js";
import { BrandMark } from "./components/BrandMark.js";

export function App() {
  const commands = useCommands();
  const bootstrap = useCommandResource(
    () => commands.getBootstrap(),
    "bootstrap",
  );
  const location = useAppLocation();
  const [setupCompletedLocally, setSetupCompletedLocally] = useState(false);
  const [statusRefresh, setStatusRefresh] = useState(0);
  const refreshStatus = () => {
    bootstrap.reload();
    setStatusRefresh((value) => value + 1);
  };

  useEffect(() => {
    if (bootstrap.state !== "ready") return;
    const onboardingComplete =
      bootstrap.data.onboardingComplete || setupCompletedLocally;
    if (!onboardingComplete && location.page !== "/onboarding") {
      navigate("/onboarding");
    } else if (
      onboardingComplete &&
      location.page === "/onboarding" &&
      globalThis.location.hash === ""
    ) {
      navigate(bootstrap.data.routeHint);
    }
  }, [bootstrap, location.page, setupCompletedLocally]);

  if (bootstrap.state === "loading" && bootstrap.data === null) {
    return (
      <div className="boot-screen">
        <div className="boot-screen__mark" aria-hidden="true">
          <BrandMark />
        </div>
        <LoadingState label="Opening the local Cutokyo core" />
      </div>
    );
  }
  if (bootstrap.state === "error" && bootstrap.data === null) {
    return (
      <div className="boot-screen">
        <ErrorState
          title="Cutokyo could not start"
          error={bootstrap.error}
          onRetry={bootstrap.reload}
        />
      </div>
    );
  }
  const data = bootstrap.data;
  if (data === null) return null;

  const onboardingComplete = data.onboardingComplete || setupCompletedLocally;
  const displayData = onboardingComplete
    ? { ...data, onboardingComplete: true }
    : data;
  const effectivePage = onboardingComplete ? location.page : "/onboarding";
  return (
    <AppShell
      bootstrap={displayData}
      currentPath={effectivePage}
      onRecheckStatus={refreshStatus}
      recheckingStatus={bootstrap.state === "loading"}
    >
      {bootstrap.state === "error" ? (
        <ErrorState
          title="Local status could not refresh"
          error={bootstrap.error}
          onRetry={refreshStatus}
        />
      ) : null}
      {effectivePage === "/onboarding" ? (
        <OnboardingPage
          mode={onboardingComplete ? "management" : "onboarding"}
          statusRefresh={statusRefresh}
          onComplete={() => {
            setSetupCompletedLocally(true);
            bootstrap.reload();
            navigate("/dashboard");
          }}
        />
      ) : effectivePage === "/dashboard" ? (
        <DashboardPage />
      ) : effectivePage === "/sessions" ? (
        <SessionsPage sessionId={location.sessionId} />
      ) : effectivePage === "/inventory" ? (
        <InventoryPage />
      ) : effectivePage === "/health" ? (
        <HealthPage onHealthChange={bootstrap.reload} />
      ) : effectivePage === "/data" ? (
        <DataControlsPage />
      ) : (
        <SettingsPage statusRefresh={statusRefresh} />
      )}
    </AppShell>
  );
}
