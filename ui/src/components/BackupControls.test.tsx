import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { CommandProvider } from "../commands/context.js";
import type {
  BackupInfo,
  BackupRestoreReceipt,
  CommandClient,
} from "../contracts.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { AnnouncementProvider } from "./Announcer.js";
import { BackupControls } from "./BackupControls.js";

function setup(client: CommandClient, onRestored = vi.fn()) {
  render(
    <CommandProvider client={client}>
      <AnnouncementProvider>
        <BackupControls onRestored={onRestored} />
      </AnnouncementProvider>
    </CommandProvider>,
  );
  return { user: userEvent.setup(), onRestored };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

async function openCreate(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Create backup" }));
  return screen.getByRole("dialog", { name: "Create a local backup" });
}

describe("local backup controls", () => {
  it.each([0, 1, 2])(
    "uses singular or plural session copy for count %i in list, preview, and restore receipt",
    async (count) => {
      const client = createBrowserFixtureClient("search-resume");
      const original = await client.createBackup();
      const backup = { ...original, sessionCount: count };
      const preview = await client.previewBackupRestore(original.path);
      vi.spyOn(client, "listBackups").mockResolvedValue([backup]);
      vi.spyOn(client, "previewBackupRestore").mockResolvedValue({
        ...preview,
        backup,
        currentSessionCount: count,
      });
      vi.spyOn(client, "restoreBackup").mockResolvedValue({
        recoveryPath: "/isolated-cutokyo/backups/recovery-grammar",
        restoredSessionCount: count,
        integrityResult: "ok",
      });
      const { user } = setup(client);
      const word = count === 1 ? "session" : "sessions";
      expect(
        await screen.findByText(new RegExp(`^${count} ${word} ·`)),
      ).toBeTruthy();
      await user.click(screen.getByRole("button", { name: "Restore…" }));
      const dialog = screen.getByRole("dialog", {
        name: "Restore this backup?",
      });
      expect(
        await within(dialog).findByText(
          `${count} current ${word} will be replaced by ${count} backed-up ${word}. The previous history will remain in a recovery copy.`,
        ),
      ).toBeTruthy();
      await user.click(
        within(dialog).getByRole("button", { name: "Restore selected backup" }),
      );
      const receipt = await screen.findByRole("dialog", {
        name: "History restored",
      });
      expect(
        within(receipt).getByText(`Restored ${count} ${word}.`),
      ).toBeTruthy();
      expect(
        within(receipt).getByText("/isolated-cutokyo/backups/recovery-grammar"),
      ).toBeTruthy();
      expect(within(receipt).getByText("ok")).toBeTruthy();
      expect(
        await screen.findByText(
          `Restored ${count} ${word}. Previous history retained at /isolated-cutokyo/backups/recovery-grammar.`,
        ),
      ).toBeTruthy();
    },
  );

  it.each([
    [25, "25 B (25 bytes)"],
    [2525, "2.5 KiB (2,525 bytes)"],
    [5 * 1024 * 1024, "5.0 MiB (5,242,880 bytes)"],
  ] as const)(
    "shows adaptive nonzero units and exact bytes for a %i-byte backup",
    async (byteLength, expected) => {
      const client = createBrowserFixtureClient("search-resume");
      const backup = await client.createBackup();
      vi.spyOn(client, "createBackup").mockResolvedValue({
        ...backup,
        byteLength,
      });
      const { user } = setup(client);
      const dialog = await openCreate(user);
      await user.click(
        within(dialog).getByRole("button", { name: "Create backup" }),
      );
      const receipt = await screen.findByRole("dialog", {
        name: "Backup created",
      });
      expect(within(receipt).getByText(expected)).toBeTruthy();
      expect(within(receipt).queryByText(/^0\.00 MB/)).toBeNull();
    },
  );

  it("creates with one default action and shows actual location, size, date, scope, and available backup", async () => {
    const client = createBrowserFixtureClient("search-resume");
    const create = vi.spyOn(client, "createBackup");
    const { user } = setup(client);
    await screen.findByText(/No backups yet/);
    const dialog = await openCreate(user);
    await user.click(
      within(dialog).getByRole("button", { name: "Create backup" }),
    );
    await screen.findByRole("dialog", { name: "Backup created" });
    expect(create).toHaveBeenCalledExactlyOnceWith(undefined);
    const receipt = screen.getByRole("dialog");
    expect(
      within(receipt).getByText("/isolated-cutokyo/backups/backup-1"),
    ).toBeTruthy();
    expect(within(receipt).getByText("Size")).toBeTruthy();
    expect(within(receipt).getByText("Created")).toBeTruthy();
    await user.click(within(receipt).getByRole("button", { name: "Done" }));
    expect(
      await screen.findByRole("button", { name: "Restore…" }),
    ).toBeTruthy();
  });

  it("cancels creation and restore review without sending any mutation", async () => {
    const client = createBrowserFixtureClient("search-resume");
    await client.createBackup();
    const create = vi.spyOn(client, "createBackup");
    const restore = vi.spyOn(client, "restoreBackup");
    const { user } = setup(client);
    const createDialog = await openCreate(user);
    await user.click(
      within(createDialog).getByRole("button", { name: "Cancel" }),
    );
    expect(create).not.toHaveBeenCalled();
    await user.click(await screen.findByRole("button", { name: "Restore…" }));
    const restoreDialog = await screen.findByRole("dialog", {
      name: "Restore this backup?",
    });
    await within(restoreDialog).findByText(
      "3 current sessions will be replaced by 3 backed-up sessions. The previous history will remain in a recovery copy.",
    );
    await user.click(
      within(restoreDialog).getByRole("button", {
        name: "Cancel — keep current history",
      }),
    );
    expect(restore).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("blocks duplicate creates and all dismissal while busy, retaining errors inside the active modal", async () => {
    const client = createBrowserFixtureClient("search-resume");
    const request = deferred<BackupInfo>();
    const create = vi
      .spyOn(client, "createBackup")
      .mockReturnValue(request.promise);
    const { user } = setup(client);
    const dialog = await openCreate(user);
    await user.dblClick(
      within(dialog).getByRole("button", { name: "Create backup" }),
    );
    expect(create).toHaveBeenCalledTimes(1);
    expect(
      (
        within(dialog).getByRole("button", {
          name: "Cancel",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (
        within(dialog).getByRole("button", {
          name: "Close dialog",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBeTruthy();
    request.reject(
      new Error("Destination already exists. Choose a new location."),
    );
    expect(await within(dialog).findByRole("alert")).toHaveProperty(
      "textContent",
      "Destination already exists. Choose a new location.",
    );
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(
      (
        within(dialog).getByRole("button", {
          name: "Cancel",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
  });

  it("restores a selected backup with one confirmation, reloads genuine fixture history, and reports retained recovery", async () => {
    const client = createBrowserFixtureClient("search-resume");
    const backup = await client.createBackup();
    await client.deleteAll("DELETE ALL LOCAL HISTORY");
    const restore = vi.spyOn(client, "restoreBackup");
    const { user, onRestored } = setup(client);
    await user.click(await screen.findByRole("button", { name: "Restore…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Restore this backup?",
    });
    await within(dialog).findByText(
      new RegExp(
        `0 current sessions will be replaced by ${backup.sessionCount}`,
      ),
    );
    await user.click(
      within(dialog).getByRole("button", { name: "Restore selected backup" }),
    );
    const receipt = await screen.findByRole("dialog", {
      name: "History restored",
    });
    expect(restore).toHaveBeenCalledTimes(1);
    expect(onRestored).toHaveBeenCalledTimes(1);
    expect(within(receipt).getByText("Recovery copy")).toBeTruthy();
    expect(
      within(receipt).getByText(/\/isolated-cutokyo\/backups\/recovery-/),
    ).toBeTruthy();
    expect(within(receipt).getByText("ok")).toBeTruthy();
    expect(
      (await client.previewSessionDeletion("session-claude-73A9")).sessionIds
        .length,
    ).toBe(1);
  });

  it("blocks duplicate restores and dismissal and shows verification failure without reporting success", async () => {
    const client = createBrowserFixtureClient("search-resume");
    await client.createBackup();
    const request = deferred<BackupRestoreReceipt>();
    const restore = vi
      .spyOn(client, "restoreBackup")
      .mockReturnValue(request.promise);
    const { user, onRestored } = setup(client);
    await user.click(await screen.findByRole("button", { name: "Restore…" }));
    const dialog = screen.getByRole("dialog");
    const apply = within(dialog).getByRole("button", {
      name: "Restore selected backup",
    });
    await waitFor(() =>
      expect((apply as HTMLButtonElement).disabled).toBe(false),
    );
    await user.dblClick(apply);
    expect(restore).toHaveBeenCalledTimes(1);
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(
      (
        within(dialog).getByRole("button", {
          name: "Cancel — keep current history",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    request.reject(
      new Error("Backup digest verification failed; history was not changed."),
    );
    expect(await within(dialog).findByRole("alert")).toHaveProperty(
      "textContent",
      "Backup digest verification failed; history was not changed.",
    );
    expect(onRestored).not.toHaveBeenCalled();
    expect(screen.queryByText("History restored")).toBeNull();
  });

  it("keeps a failed backup review in its modal and disables destructive confirmation", async () => {
    const client = createBrowserFixtureClient("search-resume");
    await client.createBackup();
    vi.spyOn(client, "previewBackupRestore").mockRejectedValue(
      new Error("Backup is corrupted. Current history remains unchanged."),
    );
    const { user } = setup(client);
    await user.click(await screen.findByRole("button", { name: "Restore…" }));
    const dialog = screen.getByRole("dialog");
    expect(await within(dialog).findByRole("alert")).toHaveProperty(
      "textContent",
      "Backup is corrupted. Current history remains unchanged.",
    );
    expect(
      (
        within(dialog).getByRole("button", {
          name: "Restore selected backup",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
  });
});
