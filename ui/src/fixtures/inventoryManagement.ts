import type {
  ActionReceipt,
  CommandClient,
  Harness,
  InventoryDocument,
  InventoryItem,
} from "../contracts.js";
import type { FixtureState } from "./scenarios.js";

const HARNESSES: readonly Harness[] = ["claude_code", "codex", "opencode"];
const ROOTS: Record<Harness, string> = {
  claude_code: ".claude",
  codex: ".codex",
  opencode: ".opencode",
};

type ManagementCommands = Pick<
  CommandClient,
  | "getInventoryDocument"
  | "saveInventoryDocument"
  | "removeInventoryItem"
  | "installInventoryItem"
  | "setInventoryItemEnabled"
>;

/** Models command state only. Native tests prove actual file mutations. */
export function createInventoryFixtureManagement(
  state: FixtureState,
): ManagementCommands {
  const documents = new Map<string, string>();
  const revisions = new Map<string, number>();
  const find = (itemId: string): InventoryItem => {
    const item = state.inventory.items.find((entry) => entry.id === itemId);
    if (item === undefined)
      throw new Error("This installation no longer exists. Refresh the list.");
    return item;
  };
  const revision = (itemId: string) =>
    `${itemId}:${revisions.get(itemId) ?? 0}`;
  const checkRevision = (itemId: string, expected: string) => {
    if (revision(itemId) !== expected) {
      throw new Error(
        "The installation changed since you opened it. Reopen it before saving.",
      );
    }
  };
  const initialContent = (item: InventoryItem) => {
    if (item.kind === "skill") {
      return `---\nname: ${item.name}\ndescription: Verify protocol messages before release.\n---\n\n# Protocol check\n\nRun the protocol fixtures and report malformed output.\n`;
    }
    if (item.kind === "instruction")
      return "# Project instructions\n\nRun tests before sharing a change.\n";
    return JSON.stringify(
      item.kind === "mcp"
        ? { command: "npx", args: ["-y", "@example/docs-mcp"], env: {} }
        : { name: item.name, command: "./scripts/check.sh" },
      null,
      2,
    );
  };
  const document = (itemId: string): InventoryDocument => {
    const item = find(itemId);
    const portable =
      item.kind === "skill" ||
      item.kind === "mcp" ||
      item.kind === "instruction";
    return {
      itemId,
      content: documents.get(itemId) ?? initialContent(item),
      format:
        item.kind === "skill" || item.kind === "instruction"
          ? "markdown"
          : "json",
      revision: revision(itemId),
      editable: !item.managedByCutokyo,
      removable: !item.managedByCutokyo,
      notes:
        item.kind === "skill"
          ? [
              "Project scope: only this repository's agents load it.",
              "Supporting files in the skill directory are copied with it.",
            ]
          : [],
      unavailableReason: item.managedByCutokyo
        ? "Cutokyo manages this capture or search entry. Use Settings or uninstall to change it."
        : null,
      installTargets: HARNESSES.map((harness) => {
        const exists = state.inventory.items.some(
          (entry) =>
            entry.name === item.name &&
            entry.kind === item.kind &&
            entry.harnesses.includes(harness),
        );
        const available = portable && !item.managedByCutokyo && !exists;
        return {
          harness,
          available,
          reason: item.managedByCutokyo
            ? "Cutokyo-owned entries are installed through setup."
            : exists
              ? "Already installed in this harness."
              : !portable
                ? "This native format cannot be translated to this harness."
                : null,
          ...(item.kind === "mcp" && harness === "codex"
            ? {
                droppedFields: ["timeout_ms", "headers.X-Docs-Region"],
                conversions: ["env is written as an [env] table"],
              }
            : {}),
          destination:
            item.kind === "instruction" && item.scope === "project"
              ? `Project/${harness === "claude_code" ? "CLAUDE.md" : "AGENTS.md"}`
              : `${item.scope === "project" ? "Project" : "Home"}/${ROOTS[harness]}/${item.kind === "skill" ? `skills/${item.name}/SKILL.md` : item.kind === "instruction" ? (harness === "claude_code" ? "CLAUDE.md" : "AGENTS.md") : "native MCP configuration"}`,
        };
      }),
    };
  };
  const success = (message: string): ActionReceipt => ({
    ok: true,
    status: "success",
    message,
  });

  return {
    async getInventoryDocument(itemId) {
      return structuredClone(document(itemId));
    },
    async saveInventoryDocument(itemId, expected, content) {
      const source = document(itemId);
      checkRevision(itemId, expected);
      if (!source.editable)
        throw new Error(
          source.unavailableReason ?? "This installation is read-only.",
        );
      if (source.format === "json") JSON.parse(content);
      if (content.trim() === "")
        throw new Error("Source content must not be empty.");
      documents.set(itemId, content);
      revisions.set(itemId, (revisions.get(itemId) ?? 0) + 1);
      return success(
        `Saved ${find(itemId).name}. A recovery copy was retained.`,
      );
    },
    async removeInventoryItem(itemId, expected) {
      const source = document(itemId);
      checkRevision(itemId, expected);
      if (!source.removable)
        throw new Error(
          source.unavailableReason ??
            "This installation cannot be removed here.",
        );
      const item = find(itemId);
      state.inventory = {
        ...state.inventory,
        items: state.inventory.items.filter((entry) => entry.id !== itemId),
      };
      documents.delete(itemId);
      revisions.delete(itemId);
      return success(
        `Removed ${item.name} from this harness. Other installations are unchanged.`,
      );
    },
    async setInventoryItemEnabled(itemId, expected, enabled) {
      checkRevision(itemId, expected);
      const item = find(itemId);
      if (item.kind !== "mcp")
        throw new Error("Only MCP servers can be turned on or off");
      if (item.managedByCutokyo)
        throw new Error("Turn Cutokyo search on or off in Settings.");
      state.inventory = {
        ...state.inventory,
        items: state.inventory.items.map((entry) =>
          entry.id === itemId
            ? { ...entry, state: enabled ? "configured" : "disabled" }
            : entry,
        ),
      };
      revisions.set(itemId, (revisions.get(itemId) ?? 0) + 1);
      return success(
        enabled
          ? "Turned on. Restart the agent to load it."
          : "Turned off. Its configuration is kept.",
      );
    },
    async installInventoryItem(itemId, expected, harness) {
      const source = document(itemId);
      checkRevision(itemId, expected);
      const target = source.installTargets.find(
        (entry) => entry.harness === harness,
      );
      if (!target?.available)
        throw new Error(target?.reason ?? "Destination is unavailable.");
      const item = find(itemId);
      const id = `${itemId}-copy-${harness}`;
      state.inventory = {
        ...state.inventory,
        items: [
          ...state.inventory.items,
          {
            ...item,
            id,
            harnesses: [harness],
            origin: target.destination,
            managedByCutokyo: false,
          },
        ],
      };
      documents.set(id, source.content);
      return success(
        `Installed ${item.name} in ${harness === "claude_code" ? "Claude Code" : harness === "codex" ? "Codex" : "OpenCode"}.`,
      );
    },
  };
}
