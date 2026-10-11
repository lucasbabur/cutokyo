import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { getCutokyoGitHubStars } from "@/infrastructure/github";

import { GitHubLink } from "./github-link";

vi.mock("@/infrastructure/github", () => ({ getCutokyoGitHubStars: vi.fn() }));
const getStars = vi.mocked(getCutokyoGitHubStars);

beforeEach(() => {
  getStars.mockReset();
  localStorage.clear();
  vi.spyOn(document, "hidden", "get").mockReturnValue(false);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("GitHub star count", () => {
  it("keeps a verified number visible when the first GitHub request is rate limited", async () => {
    getStars.mockRejectedValue(new Error("GitHub repository request failed with 403"));
    render(<GitHubLink locale="en" />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole("link", { name: /1 stars/ })).toHaveTextContent("1");
  });

  it("restores the last successful count after a reload while GitHub is unavailable", async () => {
    getStars.mockResolvedValueOnce(42);
    const { unmount } = render(<GitHubLink locale="en" />);
    expect(await screen.findByRole("link", { name: /42 stars/ })).toHaveTextContent("42");
    unmount();

    getStars.mockRejectedValue(new Error("Rate limited"));
    render(<GitHubLink locale="en" />);
    expect(await screen.findByRole("link", { name: /42 stars/ })).toHaveTextContent("42");
  });

  it("refreshes the exact count when returning after starring the repository", async () => {
    getStars.mockResolvedValueOnce(0).mockResolvedValueOnce(1);
    render(<GitHubLink locale="en" />);
    expect(
      await screen.findByRole("link", { name: "View Cutokyo on GitHub · 0 stars" }),
    ).toBeVisible();

    fireEvent.focus(window);

    expect(
      await screen.findByRole("link", { name: "View Cutokyo on GitHub · 1 stars" }),
    ).toHaveTextContent("1");
    expect(getStars).toHaveBeenCalledTimes(2);
  });

  it("polls only while visible and refreshes when the tab becomes visible again", async () => {
    vi.useFakeTimers();
    getStars.mockResolvedValue(1234);
    render(<GitHubLink locale="pt-BR" />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByRole("link", { name: /1234 estrelas/ })).toHaveTextContent("1,234");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(120_000);
    });
    expect(getStars).toHaveBeenCalledTimes(2);

    vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(240_000);
    });
    fireEvent.focus(window);
    expect(getStars).toHaveBeenCalledTimes(2);

    vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    getStars.mockResolvedValue(1235);
    fireEvent(document, new Event("visibilitychange"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByRole("link", { name: /1235 estrelas/ })).toHaveTextContent("1,235");
  });

  it("preserves the last real count on failure and retries when the network returns", async () => {
    getStars
      .mockResolvedValueOnce(1)
      .mockRejectedValueOnce(new Error("Offline"))
      .mockResolvedValueOnce(2);
    render(<GitHubLink locale="en" />);
    expect(await screen.findByRole("link", { name: /1 stars/ })).toBeVisible();
    fireEvent.focus(window);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole("link", { name: /1 stars/ })).toBeVisible();

    fireEvent(window, new Event("online"));
    expect(await screen.findByRole("link", { name: /2 stars/ })).toBeVisible();
  });

  it("deduplicates overlapping events and aborts the request on unmount", () => {
    getStars.mockReturnValue(new Promise(() => undefined));
    const { unmount } = render(<GitHubLink locale="en" />);
    fireEvent.focus(window);
    fireEvent(document, new Event("visibilitychange"));
    expect(getStars).toHaveBeenCalledTimes(1);
    const signal = getStars.mock.calls[0]?.[0];
    unmount();
    expect(signal?.aborted).toBe(true);
    fireEvent.focus(window);
    expect(getStars).toHaveBeenCalledTimes(1);
  });
});
