import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import prettier from "eslint-config-prettier";
import boundaries from "eslint-plugin-boundaries";
import importPlugin from "eslint-plugin-import";
import testingLibrary from "eslint-plugin-testing-library";
import unusedImports from "eslint-plugin-unused-imports";
import globals from "globals";
import tseslint from "typescript-eslint";

import localPlugin from "./eslint/local-plugin.mjs";

const ROOT_DIR = fileURLToPath(new URL(".", import.meta.url));
const TS_PROJECT = path.join(ROOT_DIR, "tsconfig.eslint.json");

const nextConventionFiles = [
  "src/middleware.ts",
  "src/proxy.ts",
  "src/app/**/default.tsx",
  "src/app/**/error.tsx",
  "src/app/**/global-error.tsx",
  "src/app/**/layout.tsx",
  "src/app/**/loading.tsx",
  "src/app/**/not-found.tsx",
  "src/app/**/page.tsx",
  "src/app/**/template.tsx",
];

const baseRestrictedImports = {
  paths: [
    {
      message: "Use `@/infrastructure/fastapi/commands.ts` instead.",
      name: "axios",
    },
  ],
  patterns: [
    {
      group: ["../../*", "../../../*", "../../**", "../../../**", "../../../../**"],
      message: "Use `@/` aliases instead of deep relative imports.",
    },
    {
      group: ["@/features/*/*", "@/entities/*/*"],
      message: "Import feature and entity slices through their public `index.ts` entrypoint.",
    },
  ],
};

const frontendRestrictedImports = {
  paths: [
    ...baseRestrictedImports.paths,
    {
      message: "Only `@/infrastructure/fastapi/commands.ts` may consume the FastAPI client.",
      name: "@/infrastructure/fastapi/client",
    },
  ],
  patterns: [
    ...baseRestrictedImports.patterns,
    {
      group: ["@/generated/fastapi/*"],
      message: "Only infrastructure adapters may import generated FastAPI schemas.",
    },
  ],
};

const viewRestrictedImports = {
  paths: frontendRestrictedImports.paths,
  patterns: [
    ...frontendRestrictedImports.patterns,
    {
      group: ["@/infrastructure/**"],
      message:
        "View files should depend on hooks, public slice APIs, or shared UI primitives - not infrastructure adapters.",
    },
  ],
};

export default defineConfig(
  globalIgnores([
    ".next/**",
    ".lighthouseci/**",
    "build/**",
    "coverage/**",
    "next-env.d.ts",
    "node_modules/**",
    "out/**",
    "src-tauri/gen/**",
    "src-tauri/target/**",
    "src/generated/fastapi/schema.ts",
  ]),
  ...nextVitals,
  ...tseslint.configs.strict,
  ...tseslint.configs.stylistic,
  {
    extends: [tseslint.configs.disableTypeChecked],
    files: ["**/*.{js,mjs,cjs}"],
  },
  {
    files: ["**/*.{ts,tsx,mts}"],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
      parser: tseslint.parser,
      parserOptions: {
        tsconfigRootDir: ROOT_DIR,
      },
    },
    plugins: {
      "@typescript-eslint": tseslint.plugin,
      boundaries,
      import: importPlugin,
      local: localPlugin,
      "unused-imports": unusedImports,
    },
    rules: {
      "@typescript-eslint/consistent-type-imports": [
        "error",
        {
          fixStyle: "inline-type-imports",
          prefer: "type-imports",
        },
      ],
      "@typescript-eslint/consistent-type-definitions": ["error", "type"],
      "@typescript-eslint/no-import-type-side-effects": "error",
      "@typescript-eslint/no-unused-vars": "off",
      "import/consistent-type-specifier-style": ["error", "prefer-top-level"],
      "import/newline-after-import": ["error", { count: 1 }],
      "import/no-anonymous-default-export": "error",
      // dependency-cruiser owns circular dependency checks in `bun run depcruise`.
      // Keeping this rule inside ESLint made full lint runs exceed the 1 minute budget.
      "import/no-cycle": "off",
      "import/no-default-export": "error",
      "import/no-duplicates": "error",
      "import/no-extraneous-dependencies": [
        "error",
        {
          devDependencies: [
            "**/*.spec.{ts,tsx}",
            "**/*.test.{ts,tsx}",
            "**/*.test-fixtures.{ts,tsx}",
            ".dependency-cruiser.mjs",
            "eslint.config.mjs",
            "scripts/**/*.mjs",
            "vitest.config.mts",
            "vitest.setup.ts",
          ],
        },
      ],
      "import/no-unresolved": "error",
      "import/no-useless-path-segments": ["error", { noUselessIndex: true }],
      "import/order": [
        "error",
        {
          alphabetize: {
            caseInsensitive: true,
            order: "asc",
            orderImportKind: "asc",
          },
          groups: ["builtin", "external", "internal", ["parent", "sibling", "index"], "type"],
          "newlines-between": "always",
          pathGroups: [
            {
              group: "internal",
              pattern: "@/**",
              position: "before",
            },
          ],
        },
      ],
      "no-restricted-imports": ["error", baseRestrictedImports],
      "unused-imports/no-unused-imports": "error",
    },
    settings: {
      "import/core-modules": ["client-only", "server-only"],
      "import/resolver": {
        typescript: {
          project: TS_PROJECT,
        },
      },
    },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    rules: {
      ...boundaries.configs.strict.rules,
      "boundaries/no-ignored": "off",
      "boundaries/dependencies": [
        "error",
        {
          default: "disallow",
          rules: [
            {
              disallow: {
                from: {
                  type: "*",
                },
              },
              message:
                "Import feature and entity slices through their public `index.ts` entrypoint.",
              to: [
                { internalPath: "!index.ts", type: "feature" },
                { internalPath: "!index.ts", type: "entity" },
              ],
            },
            {
              allow: {
                to: [
                  { type: "app-file" },
                  { type: "feature" },
                  { type: "entity" },
                  { type: "shared" },
                  { type: "fastapi-commands" },
                  { type: "infrastructure" },
                ],
              },
              from: {
                type: "app-file",
              },
            },
            {
              allow: {
                to: [
                  { type: "entity" },
                  { type: "shared" },
                  { type: "fastapi-commands" },
                  { type: "infrastructure" },
                ],
              },
              from: {
                type: "feature",
              },
            },
            {
              allow: {
                to: [{ type: "shared" }, { type: "fastapi-commands" }, { type: "infrastructure" }],
              },
              from: {
                type: "entity",
              },
            },
            {
              allow: {
                to: {
                  type: "shared",
                },
              },
              from: {
                type: "shared",
              },
            },
            {
              allow: {
                to: [{ type: "shared" }, { type: "generated" }],
              },
              from: {
                type: "fastapi-client",
              },
            },
            {
              allow: {
                to: [{ type: "shared" }, { type: "generated" }, { type: "fastapi-client" }],
              },
              from: {
                type: "fastapi-commands",
              },
            },
            {
              allow: {
                to: [
                  { type: "shared" },
                  { type: "generated" },
                  { type: "fastapi-client" },
                  { type: "fastapi-commands" },
                  { type: "infrastructure" },
                ],
              },
              from: {
                type: "infrastructure",
              },
            },
            {
              allow: {
                to: {
                  type: "generated",
                },
              },
              from: {
                type: "generated",
              },
            },
          ],
        },
      ],
    },
    settings: {
      "boundaries/elements": [
        {
          mode: "full",
          pattern: "src/infrastructure/fastapi/commands.ts",
          type: "fastapi-commands",
        },
        {
          mode: "full",
          pattern: "src/infrastructure/fastapi/client.ts",
          type: "fastapi-client",
        },
        {
          mode: "full",
          pattern: "src/app/**/*.{ts,tsx}",
          type: "app-file",
        },
        {
          mode: "full",
          pattern: "src/proxy.ts",
          type: "app-file",
        },
        {
          mode: "full",
          pattern: "src/middleware.ts",
          type: "app-file",
        },
        {
          capture: ["slice"],
          mode: "folder",
          pattern: "src/features/*",
          type: "feature",
        },
        {
          capture: ["slice"],
          mode: "folder",
          pattern: "src/entities/*",
          type: "entity",
        },
        {
          capture: ["slice"],
          mode: "folder",
          pattern: "src/shared/*",
          type: "shared",
        },
        {
          capture: ["slice"],
          mode: "folder",
          pattern: "src/infrastructure/*",
          type: "infrastructure",
        },
        {
          capture: ["slice"],
          mode: "folder",
          pattern: "src/generated/*",
          type: "generated",
        },
      ],
      "boundaries/include": ["src/**/*.{ts,tsx}"],
      "boundaries/legacy-templates": false,
      "boundaries/root-path": ROOT_DIR,
      "import/core-modules": ["client-only", "server-only"],
      "import/resolver": {
        typescript: {
          project: TS_PROJECT,
        },
      },
    },
  },
  {
    files: ["src/{app,features,entities,shared}/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-globals": [
        "error",
        {
          message: "Go through `@/infrastructure/fastapi/commands.ts`.",
          name: "fetch",
        },
      ],
      "no-restricted-imports": ["error", frontendRestrictedImports],
      "no-restricted-properties": [
        "error",
        {
          message: "Go through `@/infrastructure/fastapi/commands.ts`.",
          object: "globalThis",
          property: "fetch",
        },
        {
          message: "Go through `@/infrastructure/fastapi/commands.ts`.",
          object: "window",
          property: "fetch",
        },
      ],
    },
  },
  {
    files: ["src/features/**/view.tsx", "src/entities/**/view.tsx"],
    rules: {
      "no-restricted-imports": ["error", viewRestrictedImports],
    },
  },
  {
    files: ["src/{app,features,entities}/**/*.{ts,tsx}"],
    rules: {
      "local/no-complex-business-logic": [
        "error",
        {
          maxScore: 4,
          maxStatements: 18,
        },
      ],
    },
  },
  {
    ...testingLibrary.configs["flat/react"],
    files: ["**/*.test.{ts,tsx}", "**/*.spec.{ts,tsx}"],
    languageOptions: {
      ...testingLibrary.configs["flat/react"].languageOptions,
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
    settings: {
      "import/core-modules": ["client-only", "server-only"],
    },
  },
  {
    files: [...nextConventionFiles, "next.config.ts", "vitest.config.mts"],
    rules: {
      "import/no-default-export": "off",
    },
  },
  {
    files: [".dependency-cruiser.mjs", "eslint.config.mjs", "scripts/**/*.mjs"],
    rules: {
      "import/no-default-export": "off",
    },
  },
  prettier,
);
