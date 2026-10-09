/** @type {import("dependency-cruiser").IConfiguration} */
const config = {
  forbidden: [
    {
      name: "no-circular",
      severity: "error",
      from: {},
      to: {
        circular: true,
      },
    },
    {
      name: "no-orphans",
      severity: "warn",
      from: {
        orphan: true,
        pathNot: [
          "^src/generated/",
          "^src/infrastructure/workos/static-authkit-server\\.ts$",
          "^src/infrastructure/workos/e2e-authkit-(server\\.ts|components\\.tsx)$",
          "\\.(test|spec)\\.(ts|tsx)$",
          "^vitest\\.setup\\.ts$",
          "^next-env\\.d\\.ts$",
        ],
      },
      to: {},
    },
    {
      name: "features-cannot-reach-app",
      severity: "error",
      from: {
        path: "^src/features/",
      },
      to: {
        path: "^src/app/",
      },
    },
    {
      name: "shared-stays-foundational",
      severity: "error",
      from: {
        path: "^src/shared/",
      },
      to: {
        path: "^src/(app|features|infrastructure|generated)/",
      },
    },
    {
      name: "frontend-cannot-import-fastapi-internals",
      severity: "error",
      from: {
        path: "^src/(app|features|shared)/",
      },
      to: {
        path: "^src/infrastructure/fastapi/(?!commands\\.ts$)",
      },
    },
    {
      name: "only-infrastructure-uses-generated-code",
      severity: "error",
      from: {
        pathNot: "^src/infrastructure/",
      },
      to: {
        path: "^src/generated/",
      },
    },
  ],
  options: {
    doNotFollow: {
      path: "node_modules",
    },
    includeOnly: "^src",
    tsConfig: {
      fileName: "tsconfig.json",
    },
    tsPreCompilationDeps: true,
    enhancedResolveOptions: {
      exportsFields: ["exports"],
      conditionNames: ["import", "require", "node", "default", "types"],
      extensions: [".ts", ".tsx", ".mjs", ".js", ".jsx", ".json"],
      mainFields: ["browser", "module", "main"],
    },
    reporterOptions: {
      text: {
        highlightFocused: true,
      },
    },
    skipAnalysisNotInRules: true,
  },
};

export default config;
