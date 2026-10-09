# Frontend Architecture Rules

## Ownership

- The FastAPI control plane is the authoritative owner of business state.
- WorkOS owns identity; the frontend never stores credentials itself.
- The Tauri desktop shell owns local capture and the encrypted local store.

## Frontend Read/Write Split

- Frontend reads and writes go through adapters under `src/infrastructure/`.
- Privileged or cross-tenant writes go through `@/infrastructure/fastapi/commands.ts`.
- UI layers never call FastAPI internals or generated clients directly.

## Folder Intent

- `src/app/` route composition and providers
- `src/features/` product flows and UI orchestration
- `src/entities/` read-model slices and reusable domain-facing UI
- `src/shared/` foundational UI, config, lib, and styles
- `src/generated/` generated contracts only
- `src/infrastructure/` FastAPI, WorkOS, Tauri, and local-store adapters

## Enforced Guardrails

- No raw `fetch` in `src/app`, `src/features`, `src/entities`, or `src/shared`
- No direct imports of FastAPI internals from UI-facing layers
- No deep imports into `features/*` or `entities/*`; use public `index.ts`
- Dependency boundaries checked by ESLint Boundaries and dependency-cruiser
- Complex frontend business logic blocked by a local ESLint rule
- Pre-commit runs format, lint-staged, and secret scanning
- Pre-push runs full validation, build, dependency checks, and Lighthouse on changed routes
