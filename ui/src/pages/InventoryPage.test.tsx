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

async function select(name: string) {
  await userEvent.click(await screen.findByRole("button", { name }));
  return screen.findByRole("complementary", { name });
}

async function manageSkill() {
  const panel = await select("protocol-check");
  await within(panel).findByDisplayValue(/Run the protocol fixtures/);
  return panel;
}

describe("agent tool management", () => {
  it("finds installations by name and harness, with clear empty recovery", async () => {
    const user = userEvent.setup();
    show();
    const search = await screen.findByRole("searchbox", {
      name: "Search installed tools",
    });
    await user.type(search, "protocol-check");
    expect(screen.getByRole("button", { name: "protocol-check" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "docs-73A9" })).toBeNull();
    await user.click(screen.getByRole("button", { name: /^Claude Code/ }));
    expect(
      screen.getByRole("heading", { name: "No tools match" }),
    ).toBeTruthy();
    await user.click(screen.getByRole("button", { name: /^All tools/ }));
    expect(screen.getByRole("button", { name: "protocol-check" })).toBeTruthy();
    await user.clear(search);
    expect(screen.getByRole("button", { name: "docs-73A9" })).toBeTruthy();
  });

  it("groups tools into counted kind sections with one column per harness", async () => {
    show();
    const table = await screen.findByRole("table");
    for (const name of [
      "Skills",
      "MCP servers",
      "Hooks",
      "Plugins",
      "Instructions",
    ]) {
      expect(
        within(table).getByRole("columnheader", {
          name: new RegExp(`^${name}\\s*\\d+$`),
        }),
      ).toBeTruthy();
    }
    for (const name of ["Claude Code", "Codex", "OpenCode"]) {
      expect(within(table).getByRole("columnheader", { name })).toBeTruthy();
    }
    const row = screen
      .getByRole("button", { name: "protocol-check" })
      .closest("tr")!;
    expect(within(row).getByRole("img", { name: "Installed in OpenCode" }));
    expect(
      within(row).getByRole("button", {
        name: "Install protocol-check in Claude Code",
      }),
    ).toBeTruthy();
  });

  it("starts an install from an empty harness cell", async () => {
    show();
    await userEvent.click(
      await screen.findByRole("button", { name: "Install docs-73A9 in Codex" }),
    );
    expect(
      await screen.findByRole("dialog", { name: "Install docs-73A9 to Codex" }),
    ).toBeTruthy();
  });

  it("turns an MCP server off and on without treating it as a problem", async () => {
    const client = show();
    const toggle = await screen.findByRole("switch", {
      name: "docs-73A9 enabled",
    });
    expect((toggle as HTMLInputElement).checked).toBe(true);
    const attention = screen.getByRole("button", { name: /^Needs attention/ });
    const before = attention.textContent;
    await userEvent.click(toggle);
    expect(
      await screen.findByRole("img", { name: "Turned off in Claude Code" }),
    ).toBeTruthy();
    expect(
      (
        screen.getByRole("switch", {
          name: "docs-73A9 enabled",
        }) as HTMLInputElement
      ).checked,
    ).toBe(false);
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(
      screen.getByRole("button", { name: /^Needs attention/ }).textContent,
    ).toBe(before);
    const item = (await client.getInventory()).items.find(
      (entry) => entry.id === "mcp-docs-73A9",
    );
    expect(item?.state).toBe("disabled");
    await userEvent.click(
      screen.getByRole("switch", { name: "docs-73A9 enabled" }),
    );
    const row = screen
      .getByRole("button", { name: "docs-73A9" })
      .closest("tr")!;
    expect(
      await within(row).findByRole("img", { name: "Installed in Claude Code" }),
    ).toBeTruthy();
  });

  it("clears the draft once a re-formatted save reloads, so installing stays possible", async () => {
    const base = createBrowserFixtureClient("mcp-plugin-inventory");
    show({
      ...base,
      // The native writer re-serializes JSON, so the saved file differs from the draft text.
      getInventoryDocument: async (id) => {
        const document = await base.getInventoryDocument(id);
        return {
          ...document,
          content: JSON.stringify(JSON.parse(document.content), null, 2),
        };
      },
    });
    const panel = await select("docs-73A9");
    const editor = await within(panel).findByRole("textbox", {
      name: "Source content",
    });
    fireEvent.change(editor, {
      target: { value: '{"command":"local-docs","args":[],"env":{}}' },
    });
    await userEvent.click(
      within(panel).getByRole("button", { name: "Save changes" }),
    );
    await waitFor(() =>
      expect((editor as HTMLTextAreaElement).value).toContain('\n  "command"'),
    );
    await userEvent.click(
      within(panel).getByRole("button", { name: "Install to Codex" }),
    );
    expect(
      await screen.findByRole("dialog", { name: "Install docs-73A9 to Codex" }),
    ).toBeTruthy();
  });

  it("gives Cutokyo's own search server no switch here", async () => {
    show();
    await screen.findByRole("button", { name: "Cutokyo search MCP" });
    expect(
      screen.queryByRole("switch", { name: "Cutokyo search MCP enabled" }),
    ).toBeNull();
  });

  it("never offers to copy hooks or plugins across harnesses", async () => {
    show();
    const row = (
      await screen.findByRole("button", {
        name: "PreToolUse project check",
      })
    ).closest("tr")!;
    expect(within(row).queryByRole("button", { name: /^Install / })).toBeNull();
    expect(
      within(row).getByRole("img", { name: "Not available in Codex" }),
    ).toBeTruthy();
  });

  it("saves native content and rereads the saved document", async () => {
    const client = show();
    const dialog = await manageSkill();
    expect(
      (
        within(dialog).getByRole("button", {
          name: "Save changes",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    const updated =
      "---\nname: protocol-check\ndescription: Test protocols.\n---\n\n# Updated protocol instructions\n";
    fireEvent.change(
      within(dialog).getByRole("textbox", { name: "Source content" }),
      { target: { value: updated } },
    );
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Save changes" }),
    );
    await waitFor(async () =>
      expect(
        (await client.getInventoryDocument("skill-protocol-check")).content,
      ).toBe(updated),
    );
    expect(
      screen.getByText(/Saved protocol-check/, {
        selector: ".success-message p",
      }),
    ).toBeTruthy();
  });

  it("keeps edits and errors inside the panel when a save fails", async () => {
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
  });

  it("requires a decision before discarding unsaved content", async () => {
    show();
    const dialog = await manageSkill();
    const editor = within(dialog).getByRole("textbox", {
      name: "Source content",
    });
    fireEvent.change(editor, { target: { value: "# Unsaved content" } });
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Close details" }),
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
    await userEvent.click(
      screen.getByRole("button", { name: "Close details" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Discard changes" }),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it.each(["Escape", "close button"])(
    "preserves unsaved edits when the discard confirmation is dismissed with %s",
    async (dismissal) => {
      show();
      const dialog = await manageSkill();
      const editor = within(dialog).getByRole("textbox", {
        name: "Source content",
      });
      fireEvent.change(editor, { target: { value: "# Preserve this draft" } });
      editor.focus();
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
      expect(
        await screen.findByText(/Saved protocol-check/, {
          selector: ".success-message p",
        }),
      ).toBeTruthy();
    },
  );

  it("installs an independent copy and never overwrites an existing destination", async () => {
    const client = show();
    const dialog = await manageSkill();
    const targets = within(dialog).getByRole("list", { name: "Harnesses" });
    // OpenCode already has this skill: shown with its path, with no action.
    expect(
      within(targets).queryByRole("button", { name: "Install to OpenCode" }),
    ).toBeNull();
    await userEvent.click(
      await within(targets).findByRole("button", {
        name: "Install to Claude Code",
      }),
    );
    const install = screen.getByRole("dialog", {
      name: "Install protocol-check to Claude Code",
    });
    expect(
      within(install).getByText(/Supporting files are copied/),
    ).toBeTruthy();
    await userEvent.click(
      within(install).getByRole("button", {
        name: "Install to Claude Code",
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
    expect(dialog).toBeTruthy();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Remove" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(
      (await client.getInventory()).items.some(
        (item) => item.id === "skill-protocol-check",
      ),
    ).toBe(true);
    dialog = screen.getByRole("complementary", { name: "protocol-check" });
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
          name: "Close details",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Saving…" }),
    );
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("complementary", { name: "protocol-check" })).toBe(
      dialog,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(save).toHaveBeenCalledTimes(1);
    finish?.({ ok: true, status: "success", message: "Saved" });
    expect(
      await screen.findByText("Saved", { selector: ".success-message p" }),
    ).toBeTruthy();
  });

  it("explains app-owned entries rather than offering destructive controls", async () => {
    const client = show();
    const dialog = await select("Cutokyo SessionStart");
    await within(dialog).findByText(/Use Settings or uninstall/);
    expect(within(dialog).queryByRole("button", { name: "Remove" })).toBeNull();
    expect(
      within(dialog).queryByRole("button", { name: "Save changes" }),
    ).toBeNull();
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
      const dialog = await select("Cutokyo SessionStart");
      await userEvent.click(
        within(dialog).getByRole("tab", { name: "Details" }),
      );
      const link = await within(dialog).findByRole("link", {
        name: "Manage capture",
      });
      expect(link.getAttribute("href")).toBe("#/onboarding");
      expect(
        within(dialog).queryByRole("button", { name: "Remove" }),
      ).toBeNull();
      expect(
        within(dialog).queryByRole("button", { name: "Save changes" }),
      ).toBeNull();
      try {
        await userEvent.click(link);
        await waitFor(() =>
          expect(globalThis.location.hash).toBe("#/onboarding"),
        );
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
      const dialog = await select("Cutokyo SessionStart");
      await within(dialog).findByText(
        /This .*entry|This installation is read-only/,
      );
      await userEvent.click(
        within(dialog).getByRole("tab", { name: "Details" }),
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
    await screen.findByRole("button", { name: "protocol-check" });
    fail = true;
    await userEvent.click(
      screen.getByRole("button", { name: "Rescan installations" }),
    );
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Native configuration could not be read",
    );
    expect(screen.getByRole("button", { name: "protocol-check" })).toBeTruthy();
  });

  it("requires explicit acknowledgement before installing with dropped fields", async () => {
    const client = show();
    const dialog = await select("docs-73A9");
    await userEvent.click(
      await within(dialog).findByRole("button", { name: "Install to Codex" }),
    );
    const install = screen.getByRole("dialog", {
      name: "Install docs-73A9 to Codex",
    });
    expect(within(install).getByText("timeout_ms")).toBeTruthy();
    expect(
      within(install).getByText("env is written as an [env] table"),
    ).toBeTruthy();
    const confirm = within(install).getByRole("button", {
      name: "Install to Codex",
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
    await screen.findByRole("table");
    const rows = [...document.querySelectorAll<HTMLElement>("tr.tool-row")];
    expect(rows.length).toBeGreaterThan(0);
    const kinds = new Set<string>();
    for (const row of rows) {
      expect(row.textContent).not.toMatch(
        /Shared native source|Supporting asset|Discovered native files/,
      );
      expect(within(row).queryByText("Source")).toBeNull();
      kinds.add(
        row.querySelector(".tool-row__icon")!.getAttribute("aria-label")!,
      );
      expect(
        within(row).getAllByRole("img", { name: /^Installed in / }).length,
      ).toBeGreaterThan(0);
    }
    expect(kinds.size).toBeGreaterThanOrEqual(4);
  });

  it("keeps provenance and notes inside the panel's Details tab", async () => {
    show();
    const dialog = await manageSkill();
    await userEvent.click(within(dialog).getByRole("tab", { name: "Details" }));
    expect(
      within(dialog).getByText(/Supporting files in the skill/),
    ).toBeTruthy();
    expect(within(dialog).getAllByText("Source").length).toBeGreaterThan(0);
  });
});
