import path from "node:path";
import { fileURLToPath } from "node:url";

import type { NextConfig } from "next";

const rootDirectory = path.dirname(fileURLToPath(import.meta.url));
const isMobileExport = process.env.NEXT_OUTPUT === "export";
const isContainerBuild = process.env.NEXT_OUTPUT === "standalone";
const e2eAuthkitRequested = process.env.CUTOKYO_E2E_AUTHKIT === "1";
const e2eBearerToken = process.env.NEXT_PUBLIC_CUTOKYO_E2E_BEARER_TOKEN?.trim() ?? "";
const e2eAppUrl = process.env.NEXT_PUBLIC_APP_URL?.trim() ?? "";
const e2eAuthkitEnabled = e2eAuthkitRequested && e2eBearerToken.length >= 32;

if (e2eAuthkitRequested) {
  if (process.env.NODE_ENV === "production") {
    throw new Error("CUTOKYO_E2E_AUTHKIT is production-impossible");
  }
  if (!e2eAuthkitEnabled) {
    throw new Error("NEXT_PUBLIC_CUTOKYO_E2E_BEARER_TOKEN must contain at least 32 characters");
  }
  const appUrl = new URL(e2eAppUrl);
  if (appUrl.protocol !== "http:" || !["127.0.0.1", "localhost"].includes(appUrl.hostname)) {
    throw new Error("CUTOKYO_E2E_AUTHKIT requires a loopback NEXT_PUBLIC_APP_URL");
  }
}

const staticWorkOsRoot = path.join(
  rootDirectory,
  "src/infrastructure/workos/static-authkit-server.ts",
);
const staticWorkOsComponents = path.join(
  rootDirectory,
  "src/infrastructure/workos/static-authkit-components.tsx",
);
const e2eWorkOsRoot = path.join(rootDirectory, "src/infrastructure/workos/e2e-authkit-server.ts");
const e2eWorkOsComponents = path.join(
  rootDirectory,
  "src/infrastructure/workos/e2e-authkit-components.tsx",
);
const workOsWebpackAliases = isMobileExport
  ? {
      "@workos-inc/authkit-nextjs/components": staticWorkOsComponents,
      "@workos-inc/authkit-nextjs$": staticWorkOsRoot,
    }
  : e2eAuthkitEnabled
    ? {
        "@workos-inc/authkit-nextjs/components": e2eWorkOsComponents,
        "@workos-inc/authkit-nextjs$": e2eWorkOsRoot,
      }
    : {};
const workOsTurbopackAliases: NonNullable<NextConfig["turbopack"]>["resolveAlias"] = isMobileExport
  ? {
      "@workos-inc/authkit-nextjs": "./src/infrastructure/workos/static-authkit-server.ts",
      "@workos-inc/authkit-nextjs/components":
        "./src/infrastructure/workos/static-authkit-components.tsx",
    }
  : e2eAuthkitEnabled
    ? {
        "@workos-inc/authkit-nextjs": "./src/infrastructure/workos/e2e-authkit-server.ts",
        "@workos-inc/authkit-nextjs/components":
          "./src/infrastructure/workos/e2e-authkit-components.tsx",
      }
    : {};
const webSecurityHeaders = [
  { key: "Permissions-Policy", value: "camera=(), geolocation=(), microphone=()" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "Strict-Transport-Security", value: "max-age=31536000; includeSubDomains" },
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "X-Frame-Options", value: "DENY" },
];

const nextConfig: NextConfig = {
  ...(isMobileExport
    ? {
        images: {
          unoptimized: true,
        },
        output: "export",
      }
    : {}),
  ...(!isMobileExport
    ? {
        async headers() {
          return [{ headers: webSecurityHeaders, source: "/:path*" }];
        },
      }
    : {}),
  ...(isContainerBuild ? { output: "standalone" } : {}),
  poweredByHeader: false,
  reactStrictMode: true,
  turbopack: {
    resolveAlias: workOsTurbopackAliases,
    root: rootDirectory,
  },
  typedRoutes: true,
  webpack(config) {
    if (isMobileExport || e2eAuthkitEnabled) {
      config.resolve.alias = {
        ...(config.resolve.alias ?? {}),
        ...workOsWebpackAliases,
      };
    }
    return config;
  },
};

export default nextConfig;
