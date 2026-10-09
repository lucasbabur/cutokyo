# Frontend Harness Notes

This folder owns the Next.js app. Keep feature code under `src/features`, shared primitives under `src/shared`, adapters under `src/infrastructure`, and generated API types under `src/generated`.

Harness expectations:

- Use `frontend-implementation` for code changes and `frontend-browser-qa` for browser evidence.
- Treat visual or interaction changes as incomplete until browser, console, and responsive checks have evidence.
- If API consumption changes, include contract evidence from `contract-guardian`.
- Keep generated OpenAPI code synchronized with backend schema changes.

Verification:

```bash
bun run validate
bun run build
```
