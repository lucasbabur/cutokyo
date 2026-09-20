# TypeScript processor example

This executable Node.js processor uses the published JSON-line protocol, declares
only `emit_derived_fact`, links output to source observations, and passes the same
bounded runtime verifier used by `cutokyo plugin verify`.

Run it from the repository root with a Node.js release that supports native
TypeScript type stripping:

```bash
cargo run -p cutokyo-cli -- plugin verify examples/plugins/ts-processor
```

The verifier gives it no database path or store handle and claims no portable
filesystem or network sandbox.
