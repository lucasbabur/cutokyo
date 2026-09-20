# Python source example

This executable source example is cursor-idempotent, emits immutable raw
observations, declares only `emit_observation`, and passes the same bounded runtime
verifier used by `cutokyo plugin verify`.

Run it from the repository root:

```bash
cargo run -p cutokyo-cli -- plugin verify examples/plugins/py-source
```

The verifier gives it no database path or store handle and claims no portable
filesystem or network sandbox.
