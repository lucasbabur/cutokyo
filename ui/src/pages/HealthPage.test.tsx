import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { CommandProvider } from "../commands/context.js";
import { AnnouncementProvider } from "../components/Announcer.js";
import type {
  ActionReceipt,
  CommandClient,
  DoctorReport,
} from "../contracts.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { HealthPage } from "./HealthPage.js";

function show(client: CommandClient) {
  render(
    <CommandProvider client={client}>
      <AnnouncementProvider>
        <HealthPage />
      </AnnouncementProvider>
    </CommandProvider>,
  );
}

async function openBundle() {
  await userEvent.click(
    await screen.findByRole("button", { name: "Export report" }),
  );
  const dialog = screen.getByRole("dialog", {
    name: "Export report",
  });
  await within(dialog).findByText("cutokyo-diagnostic-bundle.json");
  return dialog;
}

describe("health recovery and diagnostic exports", () => {
  it("shows a missing last success as Unknown, never an epoch date", async () => {
    const base = createBrowserFixtureClient("health-degraded");
    show({
      ...base,
      getHealth: async () => {
        const health = await base.getHealth();
        return {
          ...health,
          dimensions: health.dimensions.map((dimension) =>
            dimension.state === "degraded"
              ? { ...dimension, lastSuccessAt: null }
              : dimension,
          ),
        };
      },
    });
    const card = (await screen.findByText(/^writer lock$/i)).closest("article");
    expect(card).not.toBeNull();
    await userEvent.click(within(card as HTMLElement).getByText("Details"));
    const details = within(card as HTMLElement);
    expect(details.getByText("Last success").nextSibling?.textContent).toBe(
      "Unknown",
    );
    expect(document.body.textContent).not.toMatch(/1970|Dec 31, 1969/);
  });

  it("does not announce recovery when the returned dimension is still degraded", async () => {
    const base = createBrowserFixtureClient("health-degraded");
    show({ ...base, retryHealth: () => base.getHealth() });
    await userEvent.click(
      await screen.findByRole("button", { name: "Retry writer lock" }),
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("has not retried acquisition");
    expect(screen.queryByText(/writer lock recovered/)).toBeNull();
  });

  it("keeps export errors inside the active dialog and permits retry", async () => {
    const base = createBrowserFixtureClient("health-degraded");
    const create = vi
      .fn()
      .mockResolvedValueOnce({
        ok: false,
        status: "unavailable",
        message: "Export folder is not writable.",
      })
      .mockImplementation(() => base.createBundle());
    show({ ...base, createBundle: create });
    const dialog = await openBundle();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Export" }),
    );
    expect((await within(dialog).findByRole("alert")).textContent).toBe(
      "Export folder is not writable.",
    );
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Export" }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(
      screen.getByText(/App data\/diagnostics\/fixture-export/, {
        selector: ".success-message p",
      }),
    ).toBeTruthy();
    expect(create).toHaveBeenCalledTimes(2);
  });

  it("prevents dismissal and duplicate requests during an export", async () => {
    const base = createBrowserFixtureClient("health-degraded");
    let finish!: (receipt: ActionReceipt) => void;
    const create = vi.fn(
      () =>
        new Promise<ActionReceipt>((resolve) => {
          finish = resolve;
        }),
    );
    show({ ...base, createBundle: create });
    const dialog = await openBundle();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Export" }),
    );
    expect(
      (
        within(dialog).getByRole("button", {
          name: "Cancel",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    fireEvent.keyDown(document, { key: "Escape" });
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Exporting…" }),
    );
    expect(screen.getByRole("dialog")).toBe(dialog);
    expect(create).toHaveBeenCalledTimes(1);
    finish({
      ok: true,
      status: "success",
      message: "Export saved at a private local destination.",
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("does not reopen a dismissed doctor report when its result arrives", async () => {
    const base = createBrowserFixtureClient("health-degraded");
    let finish!: (report: DoctorReport) => void;
    show({
      ...base,
      runDoctor: () =>
        new Promise<DoctorReport>((resolve) => {
          finish = resolve;
        }),
    });
    await userEvent.click(
      await screen.findByRole("button", { name: "Run diagnostics" }),
    );
    const doctor = screen.getByRole("dialog", { name: "Diagnostics" });
    await userEvent.click(within(doctor).getByRole("button", { name: "Done" }));
    finish({ overall: "healthy", checks: [] });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("does not label unknown checks healthy", async () => {
    const base = createBrowserFixtureClient("populated-dashboard");
    const health = await base.getHealth();
    show({
      ...base,
      getHealth: async () => ({
        ...health,
        dimensions: health.dimensions.map((dimension) => ({
          ...dimension,
          state: "unknown",
        })),
      }),
    });
    await screen.findByText("Some checks have not run yet");
    expect(screen.queryByText("Ready")).toBeNull();
  });

  it("cancels a bundle preview without creating an export", async () => {
    const base = createBrowserFixtureClient("health-degraded");
    const create = vi.fn(() => base.createBundle());
    show({ ...base, createBundle: create });
    const dialog = await openBundle();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Cancel" }),
    );
    expect(create).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
