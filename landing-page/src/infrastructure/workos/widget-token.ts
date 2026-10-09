import { getWorkOS, withAuth } from "@workos-inc/authkit-nextjs";
import { NextResponse } from "next/server";

import { isWorkosConfigured } from "./config";
import { resolveWidgetScope } from "./widget-authorization";

import type { NextRequest } from "next/server";

export async function createWorkosWidgetToken(request: NextRequest) {
  if (!isWorkosConfigured()) {
    return NextResponse.json(
      { message: "Organization identity is not configured." },
      { status: 503 },
    );
  }
  try {
    const auth = await withAuth();
    if (!auth.user) {
      return NextResponse.json({ message: "Authentication is required." }, { status: 401 });
    }
    if (!auth.organizationId) {
      return NextResponse.json({ message: "Select an organization first." }, { status: 400 });
    }
    const requestedScope = request.nextUrl.searchParams.get("scope") ?? "users";
    const scope = resolveWidgetScope(requestedScope);
    if (!scope) {
      return NextResponse.json({ message: "Unknown identity management scope." }, { status: 400 });
    }
    if (!auth.permissions?.includes(scope)) {
      return NextResponse.json(
        { message: `The ${scope} permission is required.` },
        { status: 403 },
      );
    }
    const token = await getWorkOS().widgets.createToken({
      organizationId: auth.organizationId,
      scopes: [scope],
      userId: auth.user.id,
    });
    return NextResponse.json({ token: token.token });
  } catch {
    return NextResponse.json({ message: "Widget token creation failed." }, { status: 502 });
  }
}
