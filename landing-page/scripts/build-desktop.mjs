import { readFile, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";

const routeSwaps = [
  {
    path: "src/app/admin/account/page.tsx",
    content: [
      "export default function DesktopAccountPage() {",
      "  return <main>Account management is available in the hosted Cutokyo admin.</main>;",
      "}",
      "",
    ].join("\n"),
  },
  {
    path: "src/app/auth/callback/route.ts",
    content: [
      'import { handleAuth } from "@workos-inc/authkit-nextjs";',
      "",
      'export const dynamic = "force-static";',
      'export const GET = handleAuth({ returnPathname: "/" });',
      "",
    ].join("\n"),
  },
  {
    path: "src/app/sign-in/route.ts",
    content: [
      'import { getSignInUrl } from "@workos-inc/authkit-nextjs";',
      'import { NextResponse } from "next/server";',
      "",
      'export const dynamic = "force-static";',
      "export const GET = async () => {",
      "  const signInUrl = await getSignInUrl();",
      "  return NextResponse.redirect(signInUrl);",
      "};",
      "",
    ].join("\n"),
  },
  {
    path: "src/app/api/workos/widgets/token/route.ts",
    content: [
      'import { NextResponse } from "next/server";',
      "",
      'export const dynamic = "force-static";',
      "export const GET = () =>",
      '  NextResponse.json({ message: "Identity administration is available in the hosted admin." }, { status: 503 });',
      "",
    ].join("\n"),
  },
  {
    path: "src/app/api/workos/organizations/route.ts",
    content: [
      'import { NextResponse } from "next/server";',
      "",
      'export const dynamic = "force-static";',
      "export const GET = () =>",
      '  NextResponse.json({ message: "Organization creation is available in the hosted admin." }, { status: 503 });',
      "",
    ].join("\n"),
  },
  {
    path: "src/app/api/workos/members/route.ts",
    content: [
      'import { NextResponse } from "next/server";',
      "",
      'export const dynamic = "force-static";',
      "export const GET = () =>",
      "  NextResponse.json({ members: [] });",
      "",
    ].join("\n"),
  },
  {
    path: "src/app/api/identity/portal/route.ts",
    content: [
      'import { NextResponse } from "next/server";',
      "",
      'export const dynamic = "force-static";',
      "export const POST = () =>",
      '  NextResponse.json({ message: "Identity setup is available in the hosted admin." }, { status: 503 });',
      "",
    ].join("\n"),
  },
];

const originals = await Promise.all(
  routeSwaps.map(async (swap) => ({
    path: swap.path,
    content: await readFile(swap.path, "utf8"),
  })),
);

try {
  await Promise.all(routeSwaps.map((swap) => writeFile(swap.path, swap.content, "utf8")));
  await run("bunx", ["next", "build", "--webpack"], {
    ...process.env,
    NEXT_OUTPUT: "export",
  });
} finally {
  await Promise.all(originals.map((file) => writeFile(file.path, file.content, "utf8")));
}

function run(command, args, env) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      env,
      shell: process.platform === "win32",
      stdio: "inherit",
    });
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${command} ${args.join(" ")} failed with ${signal ?? code}`));
    });
    child.on("error", reject);
  });
}
