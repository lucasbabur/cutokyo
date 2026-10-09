import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, vi } from "vitest";

import { MarketingSiteView } from "@/features/marketing-site";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("MarketingSiteView", () => {
  it("smooth scrolls navbar section links instead of jumping to anchors", () => {
    const scrollTo = vi.spyOn(window, "scrollTo").mockImplementation(() => undefined);
    const requestAnimationFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback) => {
        callback(performance.now() + 900);
        return 1;
      });
    const pushState = vi.spyOn(window.history, "pushState");

    render(<MarketingSiteView />);

    const navigation = within(screen.getByRole("navigation"));
    expect(navigation.getByRole("link", { name: "Trusted references" })).toBeInTheDocument();
    expect(navigation.getByRole("link", { name: "Setup process" })).toBeInTheDocument();
    expect(navigation.getByRole("link", { name: "What's inside" })).toBeInTheDocument();

    fireEvent.click(navigation.getByRole("link", { name: "Trusted references" }));

    expect(requestAnimationFrame).toHaveBeenCalled();
    expect(scrollTo).toHaveBeenCalledWith(0, expect.any(Number));
    expect(pushState).toHaveBeenCalledWith(null, "", "#proof");
  });

  it("repeats negative logos and never renders fabricated desktop usage data", () => {
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    render(<MarketingSiteView />);

    expect(fetch).not.toHaveBeenCalled();

    expect(
      screen.getByRole("heading", {
        name: "Save ~50% on ai costs without changing your flow",
      }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Install, log in, cut 70% of tokens" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/compression loop/i)).not.toBeInTheDocument();
    expect(
      screen.getByText(/cutokyo is an efficiency layer between you and the model/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/built by researchers with published research in the ai space/i),
    ).toBeInTheDocument();
    expect(screen.queryByText(/we only earn/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/cutokyo takes 10%/i)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: /ai savings report stays impossible to ignore/i }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/sample desktop report/i)).not.toBeInTheDocument();
    expect(screen.queryByText("Projected AI cost saved")).not.toBeInTheDocument();
    expect(screen.queryByText("8.4m")).not.toBeInTheDocument();
    expect(screen.queryByText("$428")).not.toBeInTheDocument();
    expect(screen.queryByText("Local fallback")).not.toBeInTheDocument();
    expect(screen.queryByText("Security redactions")).not.toBeInTheDocument();
    expect(screen.queryByText("Governance log")).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/live savings breakdown/i)).not.toBeInTheDocument();
    expect(screen.getByText(/see what your model context is being used on/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /run live analysis/i })).toBeDisabled();
    expect(screen.getByRole("textbox", { name: "Context payload" })).toHaveValue("");
    expect(screen.getByText("Ready")).toBeInTheDocument();
    expect(
      screen.getByText(/nothing is submitted until you run the analysis/i),
    ).toBeInTheDocument();
    expect(screen.queryByText("operator-local")).not.toBeInTheDocument();
    expect(screen.getAllByAltText(/logo$/i)).toHaveLength(4);
    expect(screen.getAllByAltText("")).toHaveLength(10);
    expect(screen.getAllByRole("button", { name: /request demo/i })).toHaveLength(3);
    const requestDemoButton = screen.getAllByRole("button", { name: /request demo/i })[0];
    if (!requestDemoButton) throw new Error("Missing request demo button");
    requestDemoButton.focus();
    fireEvent.click(requestDemoButton);
    const dialog = within(screen.getByRole("dialog"));
    expect(screen.getByRole("dialog")).toHaveAttribute("aria-describedby", "demo-note");
    expect(dialog.getByText(/demo access is currently invite-only/i)).toBeInTheDocument();
    expect(dialog.getByRole("link", { name: "Email Cutokyo" })).toHaveAttribute(
      "href",
      "mailto:hello@cutokyo.com?subject=Cutokyo%20demo%20request",
    );
    const closeButton = dialog.getByRole("button", { name: /close demo/i });
    const contactLink = dialog.getByRole("link", { name: "Email Cutokyo" });
    expect(closeButton).toHaveFocus();
    fireEvent.keyDown(document, { key: "Tab", shiftKey: true });
    expect(contactLink).toHaveFocus();
    fireEvent.keyDown(document, { key: "Tab" });
    expect(closeButton).toHaveFocus();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(requestDemoButton).toHaveFocus();
    expect(screen.queryByLabelText(/animated scissors/i)).not.toBeInTheDocument();
    expect(screen.queryByAltText(/person working at a laptop with notes/i)).not.toBeInTheDocument();
    expect(screen.queryByAltText(/abstract light trails on a dark desk/i)).not.toBeInTheDocument();
    expect(
      screen.queryByAltText(/desktop workstation with code and documents/i),
    ).not.toBeInTheDocument();
  });

  it("renders the Brazilian Portuguese copy when requested", () => {
    render(<MarketingSiteView locale="pt-BR" />);

    expect(
      screen.getByRole("heading", {
        name: "Economize ~50% em custos de IA sem mudar seu fluxo",
      }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Instale, faça login, corte 70% dos tokens" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/A Cutokyo é uma camada de eficiência/i)).toBeInTheDocument();
    expect(
      screen.getByText(/criada por pesquisadores com pesquisas publicadas na área de ia/i),
    ).toBeInTheDocument();
    expect(screen.queryByText(/só ganhamos/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/fica com 10%/i)).not.toBeInTheDocument();
    expect(screen.queryByText("Custo de IA economizado")).not.toBeInTheDocument();
    expect(screen.queryByText("Economia estimada")).not.toBeInTheDocument();
    const solicitarDemoButton = screen.getAllByRole("button", { name: /solicitar demo/i })[0];
    if (!solicitarDemoButton) throw new Error("Missing solicitar demo button");
    fireEvent.click(solicitarDemoButton);
    const dialog = within(screen.getByRole("dialog"));
    expect(dialog.getByText(/a demo está disponível apenas por convite/i)).toBeInTheDocument();
    expect(dialog.getByRole("link", { name: "Enviar email para a Cutokyo" })).toHaveAttribute(
      "href",
      "mailto:hello@cutokyo.com?subject=Cutokyo%20demo%20request",
    );
    expect(screen.queryByText(/proxy/i)).not.toBeInTheDocument();
  });

  it("does not render invented governance or context data before the live API responds", () => {
    render(<MarketingSiteView />);

    expect(screen.getByRole("button", { name: /run live analysis/i })).toBeDisabled();
    expect(screen.getByRole("textbox", { name: "Context payload" })).toHaveValue("");
    expect(screen.getByText("Ready")).toBeInTheDocument();
    expect(screen.queryByText("operator-local")).not.toBeInTheDocument();
    expect(screen.queryByText("Current user request")).not.toBeInTheDocument();
    expect(screen.queryByText("Tool results")).not.toBeInTheDocument();
  });

  it("submits only user-supplied context with honest analysis metadata", async () => {
    const fetch = vi.fn().mockResolvedValue(new Response("{}", { status: 503 }));
    vi.stubGlobal("fetch", fetch);
    render(<MarketingSiteView />);

    expect(fetch).not.toHaveBeenCalled();
    fireEvent.change(screen.getByRole("textbox", { name: "Context payload" }), {
      target: { value: "Analyze this real request" },
    });
    fireEvent.click(screen.getByRole("button", { name: /run live analysis/i }));

    await waitFor(() => expect(fetch).toHaveBeenCalledOnce());
    const [, init] = fetch.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(init.body))).toMatchObject({
      actor: { id: "public-analyzer", team: "public-site" },
      messages: [{ content: "Analyze this real request", role: "user" }],
      resources: [],
      target: { model: "context-analysis-only", provider: "cutokyo" },
    });
  });
});
