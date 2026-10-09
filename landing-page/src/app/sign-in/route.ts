import { NextResponse } from "next/server";

import { isWorkosConfigured } from "@/infrastructure/workos/config";
import { getAdminReturnPath } from "@/infrastructure/workos/return-path";

import type { NextRequest } from "next/server";

export const dynamic = "force-dynamic";
export const GET = async (request: NextRequest) => {
  if (!isWorkosConfigured()) {
    return NextResponse.redirect(new URL("/", "http://localhost:3000"));
  }

  const { getSignInUrl } = await import("@workos-inc/authkit-nextjs");
  const returnTo = getAdminReturnPath(request.nextUrl.searchParams.get("returnTo"));
  const reauthenticate = request.nextUrl.searchParams.get("reauth") === "google";
  const signInUrl = await getSignInUrl({
    ...(reauthenticate ? { maxAge: 0 } : {}),
    returnTo,
  });
  return NextResponse.redirect(signInUrl);
};
