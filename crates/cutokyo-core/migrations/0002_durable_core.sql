CREATE TABLE accounts (
    account_id TEXT PRIMARY KEY NOT NULL,
    harness TEXT NOT NULL,
    native_account_id TEXT,
    display_name TEXT,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL
) STRICT;

CREATE TABLE turns (
    turn_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    native_turn_id TEXT,
    sequence INTEGER,
    started_at TEXT NOT NULL,
    started_at_epoch INTEGER NOT NULL,
    ended_at TEXT,
    ended_at_epoch INTEGER,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL
) STRICT;

CREATE INDEX turns_session_idx ON turns(session_id, sequence, started_at_epoch);

CREATE TABLE tool_calls (
    tool_call_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    turn_id TEXT REFERENCES turns(turn_id) ON DELETE SET NULL,
    native_tool_call_id TEXT,
    tool_name TEXT NOT NULL,
    skill_name TEXT,
    input_json TEXT CHECK (input_json IS NULL OR json_valid(input_json)),
    output_json TEXT CHECK (output_json IS NULL OR json_valid(output_json)),
    state TEXT NOT NULL,
    started_at TEXT NOT NULL,
    started_at_epoch INTEGER NOT NULL,
    ended_at TEXT,
    ended_at_epoch INTEGER,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL,
    conflict INTEGER NOT NULL DEFAULT 0 CHECK (conflict IN (0, 1))
) STRICT;

CREATE INDEX tool_calls_session_idx ON tool_calls(session_id, started_at_epoch);
CREATE INDEX tool_calls_name_idx ON tool_calls(tool_name, session_id);
CREATE INDEX tool_calls_skill_idx ON tool_calls(skill_name, session_id);

CREATE TABLE agent_runs (
    agent_run_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    parent_agent_run_id TEXT,
    native_agent_run_id TEXT,
    agent_name TEXT,
    state TEXT NOT NULL,
    started_at TEXT NOT NULL,
    started_at_epoch INTEGER NOT NULL,
    ended_at TEXT,
    ended_at_epoch INTEGER,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL,
    conflict INTEGER NOT NULL DEFAULT 0 CHECK (conflict IN (0, 1))
) STRICT;

CREATE INDEX agent_runs_session_idx ON agent_runs(session_id, started_at_epoch);
CREATE INDEX agent_runs_name_idx ON agent_runs(agent_name, session_id);

CREATE TABLE usage_values (
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    native_usage_key TEXT NOT NULL,
    metric TEXT NOT NULL CHECK (metric IN ('input_tokens', 'output_tokens', 'cache_read_tokens', 'cache_write_tokens', 'provider_cost_micros')),
    value INTEGER NOT NULL CHECK (value >= 0),
    model TEXT,
    billing_basis TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    occurred_at_epoch INTEGER NOT NULL,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL,
    conflict INTEGER NOT NULL DEFAULT 0 CHECK (conflict IN (0, 1)),
    PRIMARY KEY (session_id, native_usage_key, metric)
) STRICT;

CREATE INDEX usage_values_session_idx ON usage_values(session_id, occurred_at_epoch);
CREATE INDEX usage_values_model_idx ON usage_values(model, occurred_at_epoch);

CREATE TABLE context_breakdowns (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    instructions_tokens INTEGER,
    user_tokens INTEGER,
    assistant_tokens INTEGER,
    tool_tokens INTEGER,
    cache_tokens INTEGER,
    remaining_tokens INTEGER,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL
) STRICT;

CREATE TABLE installation_snapshots (
    snapshot_id TEXT PRIMARY KEY NOT NULL,
    harness TEXT NOT NULL,
    captured_at TEXT NOT NULL,
    captured_at_epoch INTEGER NOT NULL,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL
) STRICT;

CREATE TABLE config_items (
    config_item_id TEXT PRIMARY KEY NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES installation_snapshots(snapshot_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('mcp', 'skill', 'hook', 'plugin')),
    native_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('enabled', 'disabled', 'degraded', 'unknown')),
    scope TEXT NOT NULL,
    origin TEXT NOT NULL,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE
) STRICT;

CREATE TABLE price_snapshots (
    price_snapshot_id TEXT PRIMARY KEY NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    currency TEXT NOT NULL,
    valid_from TEXT NOT NULL,
    valid_from_epoch INTEGER NOT NULL,
    valid_until TEXT,
    valid_until_epoch INTEGER,
    input_micros_per_million INTEGER,
    output_micros_per_million INTEGER,
    cache_read_micros_per_million INTEGER,
    cache_write_micros_per_million INTEGER,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL,
    CHECK (valid_until_epoch IS NULL OR valid_until_epoch > valid_from_epoch)
) STRICT;

CREATE INDEX price_snapshots_lookup_idx
    ON price_snapshots(provider, model, valid_from_epoch, valid_until_epoch);

CREATE TABLE quota_windows (
    quota_window_id TEXT PRIMARY KEY NOT NULL,
    account_id TEXT,
    quota_name TEXT NOT NULL,
    starts_at TEXT,
    starts_at_epoch INTEGER,
    resets_at TEXT,
    resets_at_epoch INTEGER,
    quota_limit INTEGER,
    used INTEGER,
    remaining INTEGER,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL
) STRICT;

CREATE TABLE summaries (
    summary_id TEXT PRIMARY KEY NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    attribution_json TEXT NOT NULL CHECK (json_valid(attribution_json))
) STRICT;

CREATE TABLE summary_sessions (
    summary_id TEXT NOT NULL REFERENCES summaries(summary_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    PRIMARY KEY (summary_id, session_id)
) STRICT;

CREATE TABLE spool_cursors (
    entry_key TEXT PRIMARY KEY NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('ingested', 'quarantined')),
    observation_id TEXT,
    category TEXT,
    advanced_at_epoch INTEGER NOT NULL
) STRICT;

CREATE TABLE quarantines (
    entry_key TEXT PRIMARY KEY NOT NULL,
    category TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    first_affected_observation_id TEXT,
    quarantined_at_epoch INTEGER NOT NULL,
    acknowledged_at_epoch INTEGER
) STRICT;

CREATE INDEX quarantines_current_idx
    ON quarantines(acknowledged_at_epoch, quarantined_at_epoch);

CREATE TABLE health_dimensions (
    dimension TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('healthy', 'degraded', 'unknown')),
    last_success_at_epoch INTEGER,
    last_failure_at_epoch INTEGER,
    failure_category TEXT,
    detail TEXT,
    first_affected_observation_id TEXT,
    updated_at_epoch INTEGER NOT NULL
) STRICT;

INSERT INTO health_dimensions(
    dimension, status, last_success_at_epoch, last_failure_at_epoch,
    failure_category, detail, first_affected_observation_id, updated_at_epoch
) VALUES
    ('health_persistence', 'unknown', NULL, NULL, NULL, NULL, NULL, 0),
    ('quarantine', 'healthy', 0, NULL, NULL, NULL, NULL, 0),
    ('spool_cap', 'healthy', 0, NULL, NULL, NULL, NULL, 0),
    ('spool_drain', 'unknown', NULL, NULL, NULL, NULL, NULL, 0),
    ('writer_lock', 'unknown', NULL, NULL, NULL, NULL, NULL, 0),
    ('schema', 'unknown', NULL, NULL, NULL, NULL, NULL, 0),
    ('derive', 'unknown', NULL, NULL, NULL, NULL, NULL, 0),
    ('integrity', 'unknown', NULL, NULL, NULL, NULL, NULL, 0),
    ('rebuild', 'unknown', NULL, NULL, NULL, NULL, NULL, 0),
    ('backup', 'unknown', NULL, NULL, NULL, NULL, NULL, 0),
    ('restore', 'unknown', NULL, NULL, NULL, NULL, NULL, 0);

CREATE TABLE health_state (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    current_quarantine_count INTEGER NOT NULL DEFAULT 0,
    lifetime_quarantine_count INTEGER NOT NULL DEFAULT 0,
    first_affected_observation_id TEXT,
    drain_pending_count INTEGER NOT NULL DEFAULT 0,
    drain_pending_bytes INTEGER NOT NULL DEFAULT 0,
    drain_oldest_unix INTEGER,
    drain_lag_seconds INTEGER,
    spool_cap_reason TEXT,
    writer_owner TEXT,
    schema_version INTEGER NOT NULL DEFAULT 0,
    derive_version INTEGER NOT NULL DEFAULT 0,
    last_integrity_at_epoch INTEGER,
    last_integrity_result TEXT,
    last_rebuild_at_epoch INTEGER,
    last_backup_at_epoch INTEGER,
    last_backup_digest TEXT,
    last_restore_at_epoch INTEGER,
    last_restore_digest TEXT,
    health_query_generation INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO health_state(singleton) VALUES (1);

CREATE TABLE backup_history (
    digest TEXT PRIMARY KEY NOT NULL,
    path_label TEXT NOT NULL,
    byte_length INTEGER NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    integrity_result TEXT NOT NULL
) STRICT;
