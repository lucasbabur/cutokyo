import {
  inviteWorkosOrganizationMember,
  listWorkosOrganizationMembers,
} from "@/infrastructure/workos/members";

import type { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

export async function GET() {
  return listWorkosOrganizationMembers();
}

export async function POST(request: NextRequest) {
  return inviteWorkosOrganizationMember(request);
}
