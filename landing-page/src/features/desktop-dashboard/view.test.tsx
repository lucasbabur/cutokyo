import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { providerAlignedContext } from "./model";
import { DesktopDashboardView } from "./view";
import { desktopSnapshot, localSession, operation } from "./view.test-fixtures";

import type { DesktopSnapshot } from "./model";

const snapshot = desktopSnapshot();

afterEach(() => vi.unstubAllGlobals());

describe("DesktopDashboardView", () => {
  it("keeps endpoint mutations disabled while the initial encrypted state loads", () => {
    render(<DesktopDashboardView />);

    expect(screen.getByRole("main")).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("status")).toHaveTextContent("Loading encrypted endpoint state");
    expect(screen.getByText("Checking proxy")).toBeInTheDocument();
    expect(screen.queryByText("Loading privacy state")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Enable Token compression" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Detect & connect tools" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Disconnect and restore" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Sessions" })).toBeInTheDocument();
  });

  it("normalizes estimated composition to provider-reported input tokens", () => {
    const aligned = providerAlignedContext(operation({}).attributedUsage.categories, 900);

    expect(Object.values(aligned).reduce((total, value) => total + value, 0)).toBe(900);
    expect(
      Object.values(providerAlignedContext(operation({}).attributedUsage.categories, 0)).reduce(
        (total, value) => total + value,
        0,
      ),
    ).toBe(0);
  });

  it("renders provider totals, harnesses, context categories, and privacy state", () => {
    const { container } = render(<DesktopDashboardView initialData={snapshot} />);

    expect(screen.getByText("1,500")).toBeInTheDocument();
    expect(screen.queryByText("Cached input")).not.toBeInTheDocument();
    expect(screen.queryByText("Saved tokens")).not.toBeInTheDocument();
    expect(screen.getAllByText("Codex CLI").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Claude Code").length).toBeGreaterThan(0);
    expect(screen.getByText("Context signal")).toBeInTheDocument();
    expect(screen.queryByText("Community · encrypted history")).not.toBeInTheDocument();
    expect(screen.queryByText(/community/i)).not.toBeInTheDocument();
    expect(screen.getByText("Signed in as")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign out" })).toBeInTheDocument();
    expect(screen.getByText("Export active")).toBeInTheDocument();
    expect(screen.getByText("Encrypted")).toBeInTheDocument();
    expect(screen.getByText("OFF")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Enable Token compression" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.getByLabelText("aggregate context findings")).toHaveTextContent("MCP context");
    expect(container.textContent).not.toContain("private prompt");
  });

  it("renders retention and quarantined-record degradation as unhealthy", () => {
    render(
      <DesktopDashboardView
        initialData={{
          ...snapshot,
          storage: {
            ...snapshot.storage,
            blockedRecords: 2,
            degraded: true,
            quarantinedRecords: 1,
            retentionError: "Retention failed",
          },
        }}
      />,
    );

    expect(screen.getByText("Degraded")).toBeInTheDocument();
    expect(screen.getByText(/Retention failed.*2 blocked.*1 quarantined/)).toBeInTheDocument();
  });

  it("shows investigation identifiers, timing, usage, and governed telemetry", () => {
    render(<DesktopDashboardView initialData={snapshot} />);

    const [investigateButton] = screen.getAllByText("Investigate request_test");
    expect(investigateButton).toBeDefined();
    if (investigateButton) fireEvent.click(investigateButton);

    expect(screen.getAllByText("request_test").length).toBeGreaterThan(0);
    expect(screen.getAllByText("resp_test").length).toBeGreaterThan(0);
    expect(screen.getAllByText("/v1/responses · http").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Metadata only").length).toBeGreaterThan(0);
    expect(screen.getByText("900 / 900")).toBeInTheDocument();
    expect(screen.getByText("100 / 20")).toBeInTheDocument();
    expect(screen.getByText("400 / 25")).toBeInTheDocument();
    expect(screen.getAllByText(/changedFields/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/redactionCount/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/request-audit/).length).toBeGreaterThan(0);
  });

  it("resolves exact operation correlations and loads older cursor pages", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        Response.json({
          generation: "generation-1",
          nextCursor: 17,
          operations: [operation({ requestId: "request_matched", traceId: "trace_matched" })],
        }),
      )
      .mockResolvedValueOnce(
        Response.json({
          generation: "generation-1",
          nextCursor: null,
          operations: [operation({ requestId: "request_older", traceId: "trace_older" })],
        }),
      );
    vi.stubGlobal("fetch", fetchMock);
    render(<DesktopDashboardView initialData={snapshot} />);

    fireEvent.change(screen.getByLabelText("Trace, request, or provider response ID"), {
      target: { value: "request_matched" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Find exact" }));

    expect(await screen.findByText("Investigate request_matched")).toBeInTheDocument();
    expect(fetchMock.mock.calls[0]?.[0]).toContain(
      "/cutokyo/events/page?correlationId=request_matched&limit=50",
    );

    fireEvent.click(screen.getByRole("button", { name: "Load older operations" }));

    expect(await screen.findByText("Investigate request_older")).toBeInTheDocument();
    await waitFor(() =>
      expect(fetchMock.mock.calls[1]?.[0]).toContain(
        "/cutokyo/events/page?correlationId=request_matched&cursor=17&limit=50",
      ),
    );
  });

  it("shows saved tokens only while compression is enabled", () => {
    render(
      <DesktopDashboardView
        initialData={{ ...snapshot, settings: { ...snapshot.settings, compression: true } }}
      />,
    );

    expect(screen.getByText("Saved tokens")).toBeInTheDocument();
    expect(screen.getByText("600")).toBeInTheDocument();
    expect(screen.queryByText("Cached input")).not.toBeInTheDocument();
  });

  it("opens the real plugin runtime status in the same window", () => {
    window.history.replaceState(null, "", "/desktop");
    render(<DesktopDashboardView initialData={snapshot} />);

    fireEvent.click(screen.getByRole("button", { name: "Plugins" }));

    expect(window.location.search).toBe("?view=plugins");
    expect(screen.getByText("1 active plugins")).toBeInTheDocument();
    expect(screen.getByText("context-filter")).toBeInTheDocument();
    expect(screen.getByText("context-filter.exe")).toBeInTheDocument();
    window.history.replaceState(null, "", "/");
  });

  it("announces plugin runtime errors", () => {
    window.history.replaceState(null, "", "/desktop?view=plugins");
    render(
      <DesktopDashboardView
        initialData={{
          ...snapshot,
          plugins: { ...snapshot.plugins, error: "Plugin manifest is invalid" },
        }}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent("Plugin manifest is invalid");
    window.history.replaceState(null, "", "/");
  });

  it("selects a query-addressed desktop window without an effect-driven render", () => {
    window.history.replaceState(null, "", "/desktop?view=sessions");

    render(<DesktopDashboardView initialData={snapshot} />);

    expect(screen.getByText("Every session. Every harness.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /sessions/i })).toHaveAttribute(
      "aria-current",
      "page",
    );
    window.history.replaceState(null, "", "/");
  });

  it("uses provider totals for aggregate context and brands supported providers", () => {
    window.history.replaceState(null, "", "/desktop?view=context");
    const sessions = [
      localSession(),
      localSession({
        cachedTokens: 100,
        conversationId: "conversation-claude",
        harness: "claude-code",
        id: "claude-code:conversation-claude",
        inputTokens: 400,
        models: ["claude-sonnet-4-6"],
        outputTokens: 100,
        provider: "anthropic-messages",
        title: "Claude session",
        totalTokens: 500,
      }),
      localSession({
        cachedTokens: 0,
        conversationId: "conversation-gemini",
        harness: "gemini-cli",
        id: "gemini-cli:conversation-gemini",
        inputTokens: 250,
        models: ["gemini-2.5-pro"],
        outputTokens: 50,
        provider: "gemini-generate-content",
        title: "Gemini session",
        totalTokens: 300,
      }),
    ];
    const { rerender } = render(<DesktopDashboardView initialData={{ ...snapshot, sessions }} />);

    expect(screen.getByText("Aggregate model usage")).toBeInTheDocument();
    expect(screen.getByText("1,800")).toBeInTheDocument();
    expect(screen.getByText("provider tokens across 3 sessions")).toBeInTheDocument();
    expect(screen.getByText("Normalized to input tokens")).toBeInTheDocument();

    window.history.replaceState(null, "", "/desktop?view=sessions");
    rerender(<DesktopDashboardView initialData={{ ...snapshot, sessions }} />);
    window.dispatchEvent(new Event("popstate"));

    expect(screen.getAllByLabelText("OpenAI provider").length).toBeGreaterThan(0);
    expect(screen.getAllByLabelText("Claude provider").length).toBeGreaterThan(0);
    expect(screen.getAllByLabelText("Gemini provider").length).toBeGreaterThan(0);
    window.history.replaceState(null, "", "/");
  });

  it("navigates sessions in the same window", () => {
    window.history.replaceState(null, "", "/desktop");
    const longAssistant = `Full assistant response: ${Array.from(
      { length: 24 },
      () => "implementation detail",
    ).join(" ")}`;
    const toolOutput = `Exit code: 0\nOutput:\n${Array.from(
      { length: 30 },
      (_, index) => `file-${index}.txt`,
    ).join("\n")}`;
    const withSession: DesktopSnapshot = {
      ...snapshot,
      sessions: [
        {
          cachedTokens: 400,
          context: operation({}).attributedUsage.categories,
          conversationId: "conversation-1",
          errors: 0,
          estimatedCostNanosUsd: 3_100_000,
          harness: "codex-cli",
          id: "codex-cli:conversation-1",
          inputTokens: 900,
          lastActivityAt: "2026-07-09T12:01:00.000Z",
          messages: [
            {
              content: "Please inspect the repository.",
              role: "user",
              timestamp: "2026-07-09T12:00:00.000Z",
              traceId: "0123456789abcdef0123456789abcdef",
              truncated: false,
            },
            {
              content: "I found the relevant session flow.",
              role: "assistant",
              timestamp: "2026-07-09T12:00:01.000Z",
              traceId: "0123456789abcdef0123456789abcdef",
              truncated: false,
            },
            {
              content: longAssistant,
              role: "assistant",
              timestamp: "2026-07-09T12:00:02.000Z",
              traceId: "1123456789abcdef0123456789abcdef",
              truncated: false,
            },
            {
              callId: "call-1",
              content: '{"command":"rg --files"}',
              kind: "tool_call",
              name: "shell_command",
              role: "tool",
              timestamp: "2026-07-09T12:00:03.000Z",
              traceId: "2123456789abcdef0123456789abcdef",
              truncated: false,
            },
            {
              callId: "call-1",
              content: toolOutput,
              kind: "tool_result",
              name: "shell_command",
              role: "tool",
              timestamp: "2026-07-09T12:00:04.000Z",
              traceId: "3123456789abcdef0123456789abcdef",
              truncated: false,
            },
          ],
          models: ["gpt-5.4"],
          operations: 1,
          outputTokens: 100,
          project: "cutokyo",
          provider: "openai-responses",
          reasoningTokens: 0,
          startedAt: "2026-07-09T12:00:00.000Z",
          title: "cutokyo",
          totalTokens: 1_000,
          traceIds: ["0123456789abcdef0123456789abcdef"],
          workspace: "/workspace/cutokyo",
        },
      ],
    };
    render(<DesktopDashboardView initialData={withSession} />);

    fireEvent.click(screen.getByRole("button", { name: "Sessions" }));

    expect(window.location.pathname).toBe("/desktop");
    expect(window.location.search).toBe("?view=sessions");
    expect(screen.getByText("Every session. Every harness.")).toBeInTheDocument();
    expect(screen.getByText("Please inspect the repository.")).toBeInTheDocument();
    expect(screen.getByText("I found the relevant session flow.")).toBeInTheDocument();
    const expander = screen.getByLabelText(/Expand Assistant message/);
    const fullMessage = screen.getByRole("group", { name: /Full Assistant message/ });
    expect(fullMessage).not.toHaveAttribute("open");
    fireEvent.click(expander);
    expect(fullMessage).toHaveAttribute("open");
    expect(screen.getByText(longAssistant)).toBeInTheDocument();
    expect(screen.getByText("Tool call · shell command")).toBeInTheDocument();
    expect(screen.getByText("Tool result · shell command")).toBeInTheDocument();
    const toolExpander = screen.getByLabelText(/Expand Tool result · shell command/);
    const fullToolResult = screen.getByRole("group", {
      name: /Full Tool result · shell command/,
    });
    expect(fullToolResult).not.toHaveAttribute("open");
    fireEvent.click(toolExpander);
    expect(fullToolResult).toHaveAttribute("open");
    expect(screen.getByText(/file-29\.txt/)).toBeInTheDocument();
    expect(
      screen
        .getByText("Content composition")
        .compareDocumentPosition(screen.getByText("Conversation")) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.getByLabelText("session context findings")).toHaveTextContent("MCP context");
    window.history.replaceState(null, "", "/");
  });
});
