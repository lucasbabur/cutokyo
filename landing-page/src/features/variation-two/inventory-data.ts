// Public browser-fixture inventory from the Cutokyo Agent tools screen.
export const harnesses = ["claude_code", "codex", "opencode"] as const;
export type Harness = (typeof harnesses)[number];
export const harnessNames = { claude_code: "Claude Code", codex: "Codex", opencode: "OpenCode" };
export const harnessPaths: Record<Harness, string> = {
  claude_code:
    "M21 10.5h3v3h-3v3h-1.5v3H18v-3h-1.5v3H15v-3H9v3H7.5v-3H6v3H4.5v-3H3v-3H0v-3h3v-6h18Zm-15 0h1.5v-3H6Zm10.5 0H18v-3h-1.5z",
  codex:
    "M8.086.457a6.105 6.105 0 013.046-.415c1.333.153 2.521.72 3.564 1.7a.117.117 0 00.107.029c1.408-.346 2.762-.224 4.061.366l.063.03.154.076c1.357.703 2.33 1.77 2.918 3.198.278.679.418 1.388.421 2.126a5.655 5.655 0 01-.18 1.631.167.167 0 00.04.155 5.982 5.982 0 011.578 2.891c.385 1.901-.01 3.615-1.183 5.14l-.182.22a6.063 6.063 0 01-2.934 1.851.162.162 0 00-.108.102c-.255.736-.511 1.364-.987 1.992-1.199 1.582-2.962 2.462-4.948 2.451-1.583-.008-2.986-.587-4.21-1.736a.145.145 0 00-.14-.032c-.518.167-1.04.191-1.604.185a5.924 5.924 0 01-2.595-.622 6.058 6.058 0 01-2.146-1.781c-.203-.269-.404-.522-.551-.821a7.74 7.74 0 01-.495-1.283 6.11 6.11 0 01-.017-3.064.166.166 0 00.008-.074.115.115 0 00-.037-.064 5.958 5.958 0 01-1.38-2.202 5.196 5.196 0 01-.333-1.589 6.915 6.915 0 01.188-2.132c.45-1.484 1.309-2.648 2.577-3.493.282-.188.55-.334.802-.438.286-.12.573-.22.861-.304a.129.129 0 00.087-.087A6.016 6.016 0 015.635 2.31C6.315 1.464 7.132.846 8.086.457zm-.804 7.85a.848.848 0 00-1.473.842l1.694 2.965-1.688 2.848a.849.849 0 001.46.864l1.94-3.272a.849.849 0 00.007-.854l-1.94-3.393zm5.446 6.24a.849.849 0 000 1.695h4.848a.849.849 0 000-1.696h-4.848z",
  opencode: "M22 24H2V0h20zM17 4.8H7v14.4h10z",
};
export const kinds = ["skill", "mcp", "hook", "plugin", "instruction"] as const;
export type Kind = (typeof kinds)[number];
export const kindNames = {
  skill: "Skills",
  mcp: "MCP servers",
  hook: "Hooks",
  plugin: "Plugins",
  instruction: "Instructions",
};
export type InventoryTool = {
  id: string;
  kind: Kind;
  name: string;
  description: string;
  scope: string;
  origin: string;
  harnesses: Harness[];
  managed?: boolean;
  warning?: boolean;
};
export const inventoryTools: InventoryTool[] = [
  {
    id: "protocol",
    kind: "skill",
    name: "protocol-check",
    description: "User-owned skill discovered from harness configuration.",
    scope: "project",
    origin: "Project skills directory",
    harnesses: ["opencode"],
  },
  {
    id: "search",
    kind: "mcp",
    name: "Cutokyo search MCP",
    description: "Read-only session and coverage search. It never injects context automatically.",
    scope: "managed",
    origin: "Cutokyo read-only agent surface",
    harnesses: [...harnesses],
    managed: true,
  },
  {
    id: "docs",
    kind: "mcp",
    name: "docs-73A9",
    description:
      "Documentation tools configured in Claude Code. Install a copy for another harness.",
    scope: "user",
    origin: "Claude Code user MCP configuration",
    harnesses: ["claude_code"],
  },
  {
    id: "session",
    kind: "hook",
    name: "Cutokyo SessionStart",
    description: "Atomic spool capture; the hook never opens SQLite.",
    scope: "user",
    origin: "Claude Code user settings · Cutokyo-owned entry",
    harnesses: ["claude_code"],
    managed: true,
  },
  {
    id: "check",
    kind: "hook",
    name: "PreToolUse project check",
    description: "Run a local check before selected tool calls.",
    scope: "project",
    origin: "Project/.claude/settings.json",
    harnesses: ["claude_code"],
  },
  {
    id: "processor",
    kind: "plugin",
    name: "fixture-processor-73A9",
    description: "Protocol v1 processor with transcript access denied and network access denied.",
    scope: "project",
    origin: "Project plugin manifest · synthetic fixture",
    harnesses: ["claude_code"],
  },
  {
    id: "release",
    kind: "plugin",
    name: "release-notes-indexer",
    description: "Handshake timed out. Other plugins and MCP routes continue independently.",
    scope: "project",
    origin: "Project plugin manifest",
    harnesses: ["opencode"],
    warning: true,
  },
  {
    id: "agents",
    kind: "instruction",
    name: "AGENTS.md",
    description: "Shared project instructions for Codex and OpenCode.",
    scope: "project",
    origin: "Project/AGENTS.md",
    harnesses: ["codex", "opencode"],
  },
];
export function isPortable(tool: InventoryTool) {
  return ["skill", "mcp", "instruction"].includes(tool.kind) && !tool.managed;
}
export function sourceContent(tool: InventoryTool) {
  if (tool.kind === "skill")
    return `---\nname: ${tool.name}\ndescription: Verify protocol messages before release.\n---\n\n# Protocol check\n\nRun the protocol fixtures and report malformed output.\n`;
  if (tool.kind === "instruction")
    return "# Project instructions\n\nRun tests before sharing a change.\n";
  return JSON.stringify(
    tool.kind === "mcp"
      ? { command: "npx", args: ["-y", "@example/docs-mcp"], env: {} }
      : { name: tool.name, command: "./scripts/check.sh" },
    null,
    2,
  );
}
