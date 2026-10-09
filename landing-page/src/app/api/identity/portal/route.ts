import { createIdentityPortalLink } from "@/infrastructure/workos/portal";

import type { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

export async function POST(request: NextRequest) {
  return createIdentityPortalLink(request);
}
