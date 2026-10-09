import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { CommandProvider } from "../commands/context.js";
import { AnnouncementProvider } from "../components/Announcer.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { SettingsPage } from "./SettingsPage.js";

describe("unavailable native capabilities", () => {
  it("does not offer automatic updates or a dead check when metadata is absent", async () => {
    const base = createBrowserFixtureClient("native-capabilities-unavailable");
    const check = vi.fn(base.checkForUpdates);
    const patch = vi.fn(base.patchDesktopPreferences);
    render(
      <CommandProvider
        client={{
          ...base,
          checkForUpdates: check,
          patchDesktopPreferences: patch,
        }}
      >
        <AnnouncementProvider>
          <SettingsPage />
        </AnnouncementProvider>
      </CommandProvider>,
    );
    await screen.findByText("Install updates manually for now.");
    expect(
      (
        screen.getByRole("combobox", {
          name: "Updater behavior",
        }) as HTMLSelectElement
      ).disabled,
    ).toBe(true);
    const button = screen.getByRole("button", { name: "Check now" });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(button);
    expect(check).not.toHaveBeenCalled();
    expect(patch).not.toHaveBeenCalled();
  });
});
