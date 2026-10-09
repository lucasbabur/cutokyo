import { NextResponse } from "next/server";

import { isWorkosConfigured } from "@/infrastructure/workos/config";

import type { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

export const GET = async (request: NextRequest) => {
  if (!isWorkosConfigured()) {
    return NextResponse.redirect(new URL("/", "http://localhost:3000"));
  }

  const { handleAuth } = await import("@workos-inc/authkit-nextjs");
  return handleAuth({ returnPathname: "/admin" })(request);
};
