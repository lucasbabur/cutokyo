import { createWorkosOrganization } from "@/infrastructure/workos/organizations";

import type { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

export async function POST(request: NextRequest) {
  return createWorkosOrganization(request);
}
