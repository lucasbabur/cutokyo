import { FileText, Plug, Puzzle, Sparkles, Webhook } from "lucide-react";

import type { InventoryKind } from "../contracts.js";

export const KIND_LABEL: Readonly<Record<InventoryKind, string>> = {
  skill: "Skill",
  mcp: "MCP server",
  hook: "Hook",
  plugin: "Plugin",
  instruction: "Instruction",
};

const KIND_ICON = {
  skill: Sparkles,
  mcp: Plug,
  hook: Webhook,
  plugin: Puzzle,
  instruction: FileText,
} as const;

/** One distinct symbol per kind so a mixed list scans without reading labels. */
export function KindIcon({ kind }: { readonly kind: InventoryKind }) {
  const Icon = KIND_ICON[kind];
  return <Icon aria-hidden="true" />;
}
