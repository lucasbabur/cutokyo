# ADR 0001: Local-first ownership and one core writer

- Status: Accepted
- Date: 2026-09-19

## Context

Cutokyo stores sensitive session evidence on the user's machine. SQLite WAL allows
concurrent readers but still serializes writes. Running an undisclosed daemon or
letting every frontend open write connections would obscure ownership and create
contention, lock stealing, and inconsistent recovery.

## Decision

Cutokyo is local-only by default and has no account, cloud synchronization, remote
control plane, or telemetry path. One CLI or desktop core process acquires a process
writer lock and owns every SQLite mutation. Hooks and plugins only write spool files.
CLI, desktop, MCP, and plugins invoke app ports; they never open SQLite directly.

There is no always-on daemon in v0.x. A second frontend reports or connects to the
owner. It does not infer death from a stale-looking timestamp or steal the lock.
Every connection comes from one factory that applies WAL, foreign keys,
`synchronous=NORMAL`, and a five-second busy timeout. Database paths must be local.

## Consequences

Write ownership is obvious and testable, while capture can continue with Cutokyo
closed. Backlog drains at the next owning-core startup. Multi-frontend coordination
must expose actionable ownership state. Network filesystems and cloud-synced database
paths are rejected rather than supported best-effort.
