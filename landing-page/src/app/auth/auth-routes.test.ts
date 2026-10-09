import { NextRequest } from "next/server";
import { vi } from "vitest";

import { GET as handleCallback } from "./callback/route";
import { GET as handleSignIn } from "../sign-in/route";

const { callbackHandler, getSignInUrl, handleAuth } = vi.hoisted(() => ({
  callbackHandler: vi.fn(),
  getSignInUrl: vi.fn(),
  handleAuth: vi.fn(),
}));

vi.mock("@workos-inc/authkit-nextjs", () => ({ getSignInUrl, handleAuth }));
vi.mock("@/infrastructure/workos/config", () => ({ isWorkosConfigured: () => true }));

describe("WorkOS authentication routes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getSignInUrl.mockResolvedValue("https://authkit.example.test/authorize");
    callbackHandler.mockResolvedValue(new Response(null, { status: 307 }));
    handleAuth.mockReturnValue(callbackHandler);
  });

  it("preserves the requested enterprise page in WorkOS state", async () => {
    const response = await handleSignIn(
      new NextRequest("http://localhost:3000/sign-in?returnTo=%2Fadmin%2Fenterprise"),
    );

    expect(getSignInUrl).toHaveBeenCalledWith({ returnTo: "/admin/enterprise" });
    expect(response.headers.get("location")).toBe("https://authkit.example.test/authorize");
  });

  it("rejects an external return path", async () => {
    await handleSignIn(
      new NextRequest("http://localhost:3000/sign-in?returnTo=https%3A%2F%2Fevil.example"),
    );

    expect(getSignInUrl).toHaveBeenCalledWith({ returnTo: "/admin" });
  });

  it("forces reauthentication when syncing a Google profile", async () => {
    await handleSignIn(
      new NextRequest("http://localhost:3000/sign-in?returnTo=%2Fadmin%2Faccount&reauth=google"),
    );

    expect(getSignInUrl).toHaveBeenCalledWith({ maxAge: 0, returnTo: "/admin/account" });
  });

  it("uses the admin home when WorkOS state has no requested page", async () => {
    const request = new NextRequest("http://localhost:3000/auth/callback?code=code&state=state");

    await handleCallback(request);

    expect(handleAuth).toHaveBeenCalledWith({ returnPathname: "/admin" });
    expect(callbackHandler).toHaveBeenCalledWith(request);
  });
});
