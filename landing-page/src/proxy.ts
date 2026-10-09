import { NextResponse } from "next/server";

import { isWorkosConfigured } from "@/infrastructure/workos/config";

import type { NextFetchEvent, NextMiddleware, NextRequest } from "next/server";

let workosProxy: NextMiddleware | null = null;

export default async function proxy(request: NextRequest, event: NextFetchEvent) {
  if (!isWorkosConfigured()) {
    return NextResponse.next();
  }
  return (await getWorkosProxy())(request, event);
}

export const config = {
  matcher: [
    "/",
    "/auth/callback",
    "/sign-in",
    "/((?!_next/static|_next/image|favicon.ico|.*\\.(?:svg|png|jpg|jpeg|gif|webp|ico)$).*)",
  ],
};

async function getWorkosProxy(): Promise<NextMiddleware> {
  if (!workosProxy) {
    const { authkitProxy } = await import("@workos-inc/authkit-nextjs");
    workosProxy = authkitProxy();
  }
  return workosProxy;
}
