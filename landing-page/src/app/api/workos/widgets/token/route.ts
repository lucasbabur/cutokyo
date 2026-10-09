import { createWorkosWidgetToken } from "@/infrastructure/workos/widget-token";

import type { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

export async function GET(request: NextRequest) {
  return createWorkosWidgetToken(request);
}
