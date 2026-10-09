import {
  act,
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
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { DataControlsPage } from "./DataControlsPage.js";

function setup(client = createBrowserFixtureClient("retention-delete")) {
  render(
    <CommandProvider client={client}>
      <AnnouncementProvider>
        <DataControlsPage />
      </AnnouncementProvider>
    </CommandProvider>,
  );
  return { user: userEvent.setup(), client };
}

describe("data policy and restored history", () => {
  it.each([
    [0, 0],
    [1, 1],
    [1, 2],
    [2, 1],
    [2, 2],
  ])(
    "uses independent singular/plural nouns for a retention receipt with %i sessions and %i search rows",
    async (sessions, ftsRows) => {
      const client = createBrowserFixtureClient("retention-delete");
      const applyRetention = client.applyRetention;
      vi.spyOn(client, "applyRetention").mockImplementation(async (token) => ({
        ...(await applyRetention(token)),
        sessions,
        ftsRows,
      }));
      const { user } = setup(client);
      const policy = await screen.findByRole("combobox", {
        name: "Keep sessions for",
      });
      await waitFor(() =>
        expect((policy as HTMLSelectElement).disabled).toBe(false),
      );
      await user.selectOptions(policy, "30");
      await user.click(
        screen.getByRole("button", { name: "Preview 30-day cleanup" }),
      );
      const apply = within(screen.getByRole("dialog")).getByRole("button", {
        name: "Apply exactly this preview",
      });
      await waitFor(() =>
        expect((apply as HTMLButtonElement).disabled).toBe(false),
      );
      await user.click(apply);
      const expected = `Retention applied exactly as previewed: ${sessions} ${sessions === 1 ? "session" : "sessions"} and ${ftsRows} search ${ftsRows === 1 ? "row" : "rows"} deleted.`;
      expect(await screen.findByText(expected)).toBeTruthy();
    },
  );

  it.each([1, 2])(
    "uses correct session noun when delete-all reports %i removed",
    async (sessions) => {
      const client = createBrowserFixtureClient("retention-delete");
      const deleteAll = client.deleteAll;
      vi.spyOn(client, "deleteAll").mockImplementation(
        async (confirmation) => ({
          ...(await deleteAll(confirmation)),
          sessions,
        }),
      );
      const { user } = setup(client);
      await user.click(
        await screen.findByRole("button", { name: "Delete all…" }),
      );
      const dialog = screen.getByRole("dialog");
      await user.type(
        within(dialog).getByRole("textbox"),
        "DELETE ALL LOCAL HISTORY",
      );
      await user.click(
        within(dialog).getByRole("button", {
          name: "Delete all local history",
        }),
      );
      expect(
        await screen.findByText(
          `Deleted ${sessions} local ${sessions === 1 ? "session" : "sessions"}. Existing backups were not changed.`,
        ),
      ).toBeTruthy();
      await waitFor(() =>
        expect(
          (
            screen.getByRole("button", {
              name: "Delete all…",
            }) as HTMLButtonElement
          ).disabled,
        ).toBe(true),
      );
    },
  );

  it("cancelled pending retention preview never reopens after a late result", async () => {
    const client = createBrowserFixtureClient("retention-delete");
    const preview = await client.previewRetention(30);
    let resolve!: (value: typeof preview) => void;
    const pending = new Promise<typeof preview>((finish) => {
      resolve = finish;
    });
    vi.spyOn(client, "previewRetention").mockImplementation(() => pending);
    const { user } = setup(client);
    const policy = await screen.findByRole("combobox", {
      name: "Keep sessions for",
    });
    await waitFor(() =>
      expect((policy as HTMLSelectElement).disabled).toBe(false),
    );
    await user.selectOptions(policy, "30");
    await user.click(
      screen.getByRole("button", { name: "Preview 30-day cleanup" }),
    );
    await user.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Cancel",
      }),
    );
    await act(async () => {
      resolve(preview);
      await pending;
    });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it.each(["retention", "all"] as const)(
    "%s mutation prevents duplicate apply and dismissal; failure remains in active modal",
    async (kind) => {
      const client = createBrowserFixtureClient("retention-delete");
      let reject!: (error: Error) => void;
      const failLater = () =>
        new Promise<never>((_resolve, fail) => {
          reject = fail;
        });
      const mutation =
        kind === "retention"
          ? vi.spyOn(client, "applyRetention").mockImplementation(failLater)
          : vi.spyOn(client, "deleteAll").mockImplementation(failLater);
      const { user } = setup(client);
      if (kind === "retention") {
        const policy = await screen.findByRole("combobox", {
          name: "Keep sessions for",
        });
        await waitFor(() =>
          expect((policy as HTMLSelectElement).disabled).toBe(false),
        );
        await user.selectOptions(policy, "30");
        await user.click(
          screen.getByRole("button", { name: "Preview 30-day cleanup" }),
        );
      } else {
        await user.click(
          await screen.findByRole("button", { name: "Delete all…" }),
        );
        await user.type(
          within(screen.getByRole("dialog")).getByRole("textbox"),
          "DELETE ALL LOCAL HISTORY",
        );
      }
      const dialog = screen.getByRole("dialog");
      const confirm = within(dialog).getByRole("button", {
        name:
          kind === "retention"
            ? "Apply exactly this preview"
            : "Delete all local history",
      });
      await waitFor(() =>
        expect((confirm as HTMLButtonElement).disabled).toBe(false),
      );
      act(() => {
        fireEvent.click(confirm);
        fireEvent.click(confirm);
      });
      expect(mutation).toHaveBeenCalledTimes(1);
      const cancel = within(dialog).getByRole("button", { name: /Cancel/ });
      expect((cancel as HTMLButtonElement).disabled).toBe(true);
      expect(
        (
          within(dialog).getByRole("button", {
            name: "Close dialog",
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(true);
      await user.keyboard("{Escape}");
      expect(screen.getByRole("dialog")).toBe(dialog);
      await act(async () => {
        reject(new Error("History operation failed safely"));
      });
      expect(within(dialog).getByRole("alert").textContent).toBe(
        "History operation failed safely",
      );
      expect((cancel as HTMLButtonElement).disabled).toBe(false);
      await user.click(cancel);
      expect(screen.queryByRole("dialog")).toBeNull();
    },
  );

  it("loads current retention and saves keep-until-deleted without deleting history", async () => {
    const client = createBrowserFixtureClient("retention-delete");
    await client.patchSettings({ retention_days: 90 });
    const apply = vi.spyOn(client, "applyRetention");
    const patch = vi.spyOn(client, "patchSettings");
    const { user } = setup(client);
    const policy = await screen.findByRole("combobox", {
      name: "Keep sessions for",
    });
    await waitFor(() => expect((policy as HTMLSelectElement).value).toBe("90"));
    await user.selectOptions(policy, "forever");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByText(
        /Retention policy saved: keep until explicitly deleted/,
      ),
    ).toBeTruthy();
    expect(patch).toHaveBeenCalledExactlyOnceWith({ retention_days: null });
    expect(apply).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /cleanup/i })).toBeNull();
  });

  it("applies explicit cleanup without attempting a settings write after deletion", async () => {
    const { user, client } = setup();
    const patch = vi
      .spyOn(client, "patchSettings")
      .mockRejectedValue(new Error("settings unavailable"));
    const apply = vi.spyOn(client, "applyRetention");
    const policy = await screen.findByRole("combobox", {
      name: "Keep sessions for",
    });
    await waitFor(() =>
      expect((policy as HTMLSelectElement).disabled).toBe(false),
    );
    await user.selectOptions(policy, "30");
    await user.click(
      screen.getByRole("button", { name: "Preview 30-day cleanup" }),
    );
    const modal = screen.getByRole("dialog");
    const confirm = within(modal).getByRole("button", {
      name: "Apply exactly this preview",
    });
    await waitFor(() =>
      expect((confirm as HTMLButtonElement).disabled).toBe(false),
    );
    await user.click(confirm);
    expect(
      await screen.findByText(/Retention applied exactly as previewed/),
    ).toBeTruthy();
    expect(apply).toHaveBeenCalledTimes(1);
    expect(patch).not.toHaveBeenCalled();
  });

  it("reloads the data page after actual backup restore instead of leaving the empty history view", async () => {
    const client = createBrowserFixtureClient("retention-delete");
    await client.createBackup();
    await client.deleteAll("DELETE ALL LOCAL HISTORY");
    const { user } = setup(client);
    await waitFor(() =>
      expect(
        (
          screen.getByRole("button", {
            name: "Delete all…",
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(true),
    );
    await user.click(await screen.findByRole("button", { name: "Restore…" }));
    const confirm = screen.getByRole("button", {
      name: "Restore selected backup",
    });
    await waitFor(() =>
      expect((confirm as HTMLButtonElement).disabled).toBe(false),
    );
    await user.click(confirm);
    await screen.findByRole("dialog", { name: "History restored" });
    await user.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() =>
      expect(
        (
          screen.getByRole("button", {
            name: "Delete all…",
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(false),
    );
  });
});
