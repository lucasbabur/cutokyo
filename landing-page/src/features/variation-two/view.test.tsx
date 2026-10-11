import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { getCutokyoGitHubStars } from "@/infrastructure/github";

import { VariationTwoView } from "./view";

vi.mock("@/infrastructure/github", () => ({ getCutokyoGitHubStars: vi.fn() }));

beforeEach(() => {
  vi.mocked(getCutokyoGitHubStars).mockResolvedValue(42);
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    },
  );
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("landing page", () => {
  it("links the navbar to sections and shows the live repository count", async () => {
    render(<VariationTwoView />);
    const nav = within(screen.getByRole("navigation"));
    expect(nav.getByRole("link", { name: "Integrations" })).toHaveAttribute("href", "#proof");
    expect(nav.getByRole("link", { name: "How it works" })).toHaveAttribute("href", "#how");
    expect(nav.getByRole("link", { name: "Capabilities" })).toHaveAttribute("href", "#inside");
    expect(await nav.findByRole("link", { name: /42 stars/ })).toHaveAttribute(
      "href",
      "https://github.com/lucasbabur/cutokyo",
    );
    expect(nav.queryByRole("button", { name: /request demo/i })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      "Take control of every tool in your stack.",
    );
    expect(screen.getByAltText("Claude")).toBeInTheDocument();
    expect(screen.getByAltText("OpenCode")).toBeInTheDocument();
  });

  it("offers installers and matching checksum links for every platform", async () => {
    render(<VariationTwoView />);
    await screen.findByRole("link", { name: /42 stars/ });
    const files = [
      "Cutokyo-Windows-Setup.exe",
      "Cutokyo-macOS-arm64.dmg",
      "Cutokyo-macOS-x64.dmg",
      "Cutokyo-Linux.AppImage",
      "Cutokyo-Linux.deb",
    ];
    const downloads = screen.getAllByRole("link", { name: "Download" });
    const checksums = screen.getAllByRole("link", { name: "SHA-256" });
    expect(downloads).toHaveLength(files.length);
    files.forEach((file, index) => {
      const url = `https://github.com/lucasbabur/cutokyo/releases/latest/download/${file}`;
      expect(downloads[index]).toHaveAttribute("href", url);
      expect(checksums[index]).toHaveAttribute("href", `${url}.sha256`);
    });
  });

  it("changes the capability detail when choosing a different capability", async () => {
    render(<VariationTwoView />);
    await screen.findByRole("link", { name: /42 stars/ });
    const option = screen.getByRole("button", { name: /local data control/i });
    fireEvent.click(option);
    expect(option).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("heading", { name: "Local data control" })).toBeInTheDocument();
  });

  it("renders the Portuguese route copy", async () => {
    render(<VariationTwoView locale="pt-BR" />);
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      "Um painel de controle para todos os seus agentes de IA",
    );
    expect(await screen.findByRole("link", { name: /42 estrelas/ })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Baixe a Cutokyo" })).toBeInTheDocument();
  });
});
