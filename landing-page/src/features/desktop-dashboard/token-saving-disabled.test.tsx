import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { DesktopDashboardView } from "./view";
import { desktopSnapshot } from "./view.test-fixtures";

vi.mock("@/shared/config/features", () => ({ tokenSavingEnabled: false }));

const snapshot = desktopSnapshot();

describe("desktop dashboard without token saving", () => {
  it("omits the saved tokens tile even when the setting is on", () => {
    render(
      <DesktopDashboardView
        initialData={{ ...snapshot, settings: { ...snapshot.settings, compression: true } }}
      />,
    );

    expect(screen.queryByText("Saved tokens")).not.toBeInTheDocument();
    expect(screen.queryByText("Compression")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Token compression/ })).not.toBeInTheDocument();
    // The rest of the metrics row still renders.
    expect(screen.getByText("Provider tokens")).toBeInTheDocument();
  });

  it("omits the token compression row from the controls tab", () => {
    window.history.replaceState(null, "", "/desktop?view=settings");
    render(<DesktopDashboardView initialData={snapshot} />);

    expect(screen.queryByText("Token compression")).not.toBeInTheDocument();
    expect(screen.getByText("API key redaction")).toBeInTheDocument();
    window.history.replaceState(null, "", "/");
  });
});
