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
import type { ActionReceipt, CommandClient } from "../contracts.js";
import { createBrowserFixtureClient } from "../fixtures/browserAdapter.js";
import { InventoryPage } from "./InventoryPage.js";

function show(
  client: CommandClient = createBrowserFixtureClient("mcp-plugin-inventory"),
) {
  render(
    <CommandProvider client={client}>
      <AnnouncementProvider>
        <InventoryPage />
      </AnnouncementProvider>
    </CommandProvider>,
  );
  return client;
}

async function openFileTab(dialog: HTMLElement) {
  await userEvent.click(
    await within(dialog).findByRole("tab", { name: "File" }),
  );
}

async function openInstallMenu(dialog: HTMLElement) {
  await userEvent.click(
    await within(dialog).findByRole("button", { name: /Install to…/ }),
  );
  return within(dialog).getByRole("list", { name: "Install to a harness" });
}

async function manageSkill() {
  await userEvent.click(
    await screen.findByRole("button", { name: "Manage protocol-check" }),
  );
  const dialog = await screen.findByRole("dialog", {
    name: "Manage protocol-check",
  });
  await openFileTab(dialog);
  await within(dialog).findByDisplayValue(/Run the protocol fixtures/);
  return dialog;
}

describe("agent tool management", () => {
  it("finds installations by name and harness, with clear empty recovery", async () => {
    const user = userEvent.setup();
    show();
    const search = await screen.findByRole("searchbox", {
      name: "Search installed tools",
    });
    await user.type(search, "protocol-check");
    expect(
      screen.getByRole("button", { name: "Manage protocol-check" }),
    ).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "Manage docs-73A9" }),
    ).toBeNull();
    await user.click(screen.getByRole("button", { name: "Claude Code" }));
    expect(
      screen.getByRole("heading", {
        name: "No installations match these filters",
      }),
    ).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(
      screen.getByRole("button", { name: "Manage docs-73A9" }),
    ).toBeTruthy();
  });

  it("groups installations into counted kind sections without boilerplate", async () => {
    show();
    const skills = await screen.findByRole("region", { name: /^Skills/ });
    expect(skills.textContent).toContain("1");
    expect(within(skills).getByText("protocol-check")).toBeTruthy();
    for (const name of ["MCP servers", "Hooks", "Plugins", "Instructions"]) {
      expect(
        screen.getByRole("heading", { name: new RegExp(`^${name}`) }),
      ).toBeTruthy();
    }
  });

  it("saves native content and rereads the saved document", async () => {
    const client = show();
    const dialog = await manageSkill();
    expect(
      within(dialog).queryByRole("button", { name: "Save changes" }),
    ).toBeNull();
    const updated =
      "---\nname: protocol-check\ndescription: Test protocols.\n---\n\n# Updated protocol instructions\n";
    fireEvent.change(
      within(dialog).getByRole("textbox", { name: "Source content" }),
      { target: { value: updated } },
    );
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Save changes" }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(
      (await client.getInventoryDocument("skill-protocol-check")).content,
    ).toBe(updated);
    expect(
      screen.getByText(/Saved protocol-check/, {
        selector: ".success-message p",
      }),
    ).toBeTruthy();
  });

  it("keeps edits and errors inside the active dialog when a save fails", async () => {
    const base = createBrowserFixtureClient("mcp-plugin-inventory");
    show({
      ...base,
      saveInventoryDocument: async () => {
        throw new Error("The file changed outside Cutokyo. Reopen it.");
      },
    });
    const dialog = await manageSkill();
    const editor = within(dialog).getByRole("textbox", {
      name: "Source content",
    }) as HTMLTextAreaElement;
    fireEvent.change(editor, {
      target: { value: "# Keep my unsaved instructions" },
    });
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Save changes" }),
    );
    expect((await within(dialog).findByRole("alert")).textContent).toContain(
      "file changed outside Cutokyo",
    );
    expect(editor.value).toBe("# Keep my unsaved instructions");
    expect(screen.getByRole("dialog")).toBe(dialog);
  });

  it("requires a decision before discarding unsaved content", async () => {
    show();
    const dialog = await manageSkill();
    const editor = within(dialog).getByRole("textbox", {
      name: "Source content",
    });
    fireEvent.change(editor, { target: { value: "# Unsaved content" } });
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Cancel" }),
    );
    const discard = screen.getByRole("dialog", {
      name: "Discard unsaved changes?",
    });
    await userEvent.click(
      within(discard).getByRole("button", { name: "Keep editing" }),
    );
    expect(
      (
        screen.getByRole("textbox", {
          name: "Source content",
        }) as HTMLTextAreaElement
      ).value,
    ).toBe("# Unsaved content");
    await userEvent.click(screen.getByRole("button", { name: "Close dialog" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Discard changes" }),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it.each(["Escape", "close button"])(
    "preserves unsaved edits when the discard confirmation is dismissed with %s",
    async (dismissal) => {
      show();
      const dialog = await manageSkill();
      fireEvent.change(
        within(dialog).getByRole("textbox", { name: "Source content" }),
        { target: { value: "# Preserve this draft" } },
      );
      await userEvent.keyboard("{Escape}");
      const discard = screen.getByRole("dialog", {
        name: "Discard unsaved changes?",
      });
      if (dismissal === "Escape") await userEvent.keyboard("{Escape}");
      else
        await userEvent.click(
          within(discard).getByRole("button", { name: "Close dialog" }),
        );
      expect(
        screen.queryByRole("dialog", { name: "Discard unsaved changes?" }),
      ).toBeNull();
      expect(
        (
          screen.getByRole("textbox", {
            name: "Source content",
          }) as HTMLTextAreaElement
        ).value,
      ).toBe("# Preserve this draft");
      await userEvent.click(
        screen.getByRole("button", { name: "Save changes" }),
      );
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    },
  );

  it("installs an independent copy and never overwrites an existing destination", async () => {
    const client = show();
    const dialog = await manageSkill();
    const targets = await openInstallMenu(dialog);
    // OpenCode already has this skill: shown as installed, with no action.
    expect(within(targets).getByText("Installed")).toBeTruthy();
    expect(
      within(targets).queryByRole("button", { name: "Install to OpenCode" }),
    ).toBeNull();
    await userEvent.click(
      within(targets).getByRole("button", { name: "Install to Claude Code" }),
    );
    const install = screen.getByRole("dialog", {
      name: "Install protocol-check to Claude Code",
    });
    expect(
      within(install).getByText(/Supporting files are copied/),
    ).toBeTruthy();
    await userEvent.click(
      within(install).getByRole("button", {
        name: "Confirm install to Claude Code",
      }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const items = (await client.getInventory()).items.filter(
      (item) => item.name === "protocol-check",
    );
    expect(items.map((item) => item.harnesses)).toEqual([
      ["opencode"],
      ["claude_code"],
    ]);
    const source = await client.getInventoryDocument("skill-protocol-check");
    expect(
      source.installTargets.find((target) => target.harness === "claude_code")
        ?.available,
    ).toBe(false);
    await expect(
      client.installInventoryItem(
        source.itemId,
        source.revision,
        "claude_code",
      ),
    ).rejects.toThrow("Already installed");
  });

  it("cancels removal without mutation and deletes only the chosen installation", async () => {
    const client = show();
    let dialog = await manageSkill();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Remove" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(
      (await client.getInventory()).items.some(
        (item) => item.id === "skill-protocol-check",
      ),
    ).toBe(true);
    dialog = screen.getByRole("dialog", { name: "Manage protocol-check" });
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Remove" }),
    );
    const confirm = screen.getByRole("dialog", {
      name: "Remove protocol-check?",
    });
    expect(within(confirm).getByText("Project skills directory")).toBeTruthy();
    await userEvent.click(
      within(confirm).getByRole("button", { name: "Remove installation" }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(
      (await client.getInventory()).items.some(
        (item) => item.id === "skill-protocol-check",
      ),
    ).toBe(false);
    expect(
      (await client.getInventory()).items.some(
        (item) => item.id === "mcp-docs-73A9",
      ),
    ).toBe(true);
  });

  it("does not dismiss or duplicate an in-progress write", async () => {
    const base = createBrowserFixtureClient("mcp-plugin-inventory");
    let finish: ((receipt: ActionReceipt) => void) | undefined;
    const pending = new Promise<ActionReceipt>((resolve) => {
      finish = resolve;
    });
    const save = vi.fn(() => pending);
    show({ ...base, saveInventoryDocument: save });
    const dialog = await manageSkill();
    fireEvent.change(
      within(dialog).getByRole("textbox", { name: "Source content" }),
      { target: { value: "# Updated" } },
    );
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Save changes" }),
    );
    expect(
      (
        within(dialog).getByRole("button", {
          name: "Close dialog",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBe(dialog);
    expect(save).toHaveBeenCalledTimes(1);
    finish?.({ ok: true, status: "success", message: "Saved" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("explains app-owned entries rather than offering destructive controls", async () => {
    const client = show();
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Manage Cutokyo SessionStart",
      }),
    );
    const dialog = screen.getByRole("dialog", {
      name: "Manage Cutokyo SessionStart",
    });
    await within(dialog).findByText(/Use Settings or uninstall/);
    expect(within(dialog).queryByRole("button", { name: "Remove" })).toBeNull();
    expect(
      within(dialog).queryByRole("button", { name: "Save changes" }),
    ).toBeNull();
    await openFileTab(dialog);
    expect(
      (
        within(dialog).getByRole("textbox", {
          name: "Source content",
        }) as HTMLTextAreaElement
      ).readOnly,
    ).toBe(true);
    const document = await client.getInventoryDocument("hook-session-start");
    await expect(
      client.saveInventoryDocument(
        document.itemId,
        document.revision,
        '{"command":"replace protected capture"}',
      ),
    ).rejects.toThrow(/Cutokyo manages this capture/);
    expect(await client.getInventoryDocument(document.itemId)).toEqual(
      document,
    );
  });

  it.each([
    "This is a Cutokyo-owned capture/search entry. Manage it through Cutokyo setup or Settings, not the inventory editor.",
    "Native capture is protected. Open capture management to make changes.",
    null,
  ])(
    "links protected capture hooks without depending on explanation wording: %j",
    async (unavailableReason) => {
      const originalHash = globalThis.location.hash;
      const base = createBrowserFixtureClient("mcp-plugin-inventory");
      const save = vi.fn(base.saveInventoryDocument);
      const remove = vi.fn(base.removeInventoryItem);
      const install = vi.fn(base.installInventoryItem);
      const capture = vi.fn(base.applyCaptureSetup);
      show({
        ...base,
        saveInventoryDocument: save,
        removeInventoryItem: remove,
        installInventoryItem: install,
        applyCaptureSetup: capture,
        getInventoryDocument: async (id) => ({
          ...(await base.getInventoryDocument(id)),
          unavailableReason,
        }),
      });
      const trigger = await screen.findByRole("button", {
        name: "Manage Cutokyo SessionStart",
      });
      await userEvent.click(trigger);
      const dialog = screen.getByRole("dialog");
      const link = await within(dialog).findByRole("link", {
        name: "Manage capture",
      });
      expect(link.getAttribute("href")).toBe("#/onboarding");
      expect(link.getAttribute("aria-disabled")).toBe("false");
      expect(
        within(dialog).queryByRole("button", { name: "Remove" }),
      ).toBeNull();
      expect(
        within(dialog).queryByRole("button", { name: "Save changes" }),
      ).toBeNull();
      try {
        await userEvent.click(link);
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
        await waitFor(() =>
          expect(globalThis.location.hash).toBe("#/onboarding"),
        );
        expect(document.activeElement).toBe(trigger);
        expect(save).not.toHaveBeenCalled();
        expect(remove).not.toHaveBeenCalled();
        expect(install).not.toHaveBeenCalled();
        expect(capture).not.toHaveBeenCalled();
      } finally {
        globalThis.location.hash = originalHash;
      }
    },
  );

  it.each([
    { kind: "mcp" as const, managed: true, editable: false, removable: false },
    {
      kind: "hook" as const,
      managed: false,
      editable: false,
      removable: false,
    },
    { kind: "hook" as const, managed: true, editable: true, removable: false },
    { kind: "hook" as const, managed: true, editable: false, removable: true },
  ])(
    "does not route unrelated read-only tools to capture management: %j",
    async (world) => {
      const base = createBrowserFixtureClient("mcp-plugin-inventory");
      show({
        ...base,
        getInventory: async () => {
          const inventory = await base.getInventory();
          return {
            ...inventory,
            items: inventory.items.map((item) =>
              item.id === "hook-session-start"
                ? { ...item, kind: world.kind, managedByCutokyo: world.managed }
                : item,
            ),
          };
        },
        getInventoryDocument: async (id) => ({
          ...(await base.getInventoryDocument(id)),
          editable: world.editable,
          removable: world.removable,
          unavailableReason:
            "This is a Cutokyo-owned capture/search entry. Manage it through Cutokyo setup or Settings, not the inventory editor.",
        }),
      });
      await userEvent.click(
        await screen.findByRole("button", {
          name: "Manage Cutokyo SessionStart",
        }),
      );
      const dialog = screen.getByRole("dialog");
      await within(dialog).findByText(
        /This .*entry|This installation is read-only/,
      );
      expect(
        within(dialog).queryByRole("link", { name: "Manage capture" }),
      ).toBeNull();
    },
  );

  it("shows a refresh failure even when the previous list is still visible", async () => {
    const base = createBrowserFixtureClient("mcp-plugin-inventory");
    let fail = false;
    show({
      ...base,
      getInventory: async () => {
        if (fail) throw new Error("Native configuration could not be read");
        return base.getInventory();
      },
    });
    await screen.findByRole("button", { name: "Manage protocol-check" });
    fail = true;
    await userEvent.click(
      screen.getByRole("button", { name: "Refresh installations" }),
    );
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Native configuration could not be read",
    );
    expect(
      screen.getByRole("button", { name: "Manage protocol-check" }),
    ).toBeTruthy();
  });

  it("requires explicit acknowledgement before installing with dropped fields", async () => {
    const client = show();
    await userEvent.click(
      await screen.findByRole("button", { name: "Manage docs-73A9" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Manage docs-73A9",
    });
    const targets = await openInstallMenu(dialog);
    expect(
      within(targets).getByText(/2 fields will not carry over/),
    ).toBeTruthy();
    expect(within(targets).getByText(/1 conversion/)).toBeTruthy();
    await userEvent.click(
      within(targets).getByRole("button", { name: "Install to Codex" }),
    );
    const install = screen.getByRole("dialog", {
      name: "Install docs-73A9 to Codex",
    });
    expect(within(install).getByText("timeout_ms")).toBeTruthy();
    expect(
      within(install).getByText("env is written as an [env] table"),
    ).toBeTruthy();
    const confirm = within(install).getByRole("button", {
      name: "Confirm install to Codex",
    }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);
    await userEvent.click(
      within(install).getByRole("checkbox", {
        name: "Install without these fields",
      }),
    );
    expect(confirm.disabled).toBe(false);
    await userEvent.click(confirm);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(
      (await client.getInventory()).items.some(
        (item) => item.name === "docs-73A9" && item.harnesses[0] === "codex",
      ),
    ).toBe(true);
  });

  it("shows only the item description plus kind and harness symbols in a row", async () => {
    show();
    const rows = (await screen.findAllByRole("article")) as HTMLElement[];
    expect(rows.length).toBeGreaterThan(0);
    const kinds = new Set<string>();
    for (const row of rows) {
      expect(row.textContent).not.toMatch(
        /Shared native source|Supporting asset|Discovered native files/,
      );
      expect(within(row).queryByText("Source")).toBeNull();
      kinds.add(within(row).getByRole("img").getAttribute("aria-label")!);
      expect(
        row.querySelector(".harness-chips svg.harness-mark"),
      ).not.toBeNull();
    }
    expect(kinds.size).toBeGreaterThanOrEqual(4);
  });

  it("keeps provenance and notes inside the Manage view", async () => {
    show();
    const dialog = await manageSkill();
    await userEvent.click(
      within(dialog).getByRole("tab", { name: "Overview" }),
    );
    expect(
      within(dialog).getByText(/Supporting files in the skill/),
    ).toBeTruthy();
    await userEvent.click(within(dialog).getByRole("tab", { name: "Details" }));
    expect(within(dialog).getAllByText("Source").length).toBeGreaterThan(0);
  });
});
