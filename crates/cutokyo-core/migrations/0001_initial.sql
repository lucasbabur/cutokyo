CREATE TABLE schema_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

INSERT INTO schema_meta(key, value) VALUES
    ('derive_version', '0'),
    ('retention_policy', 'keep_until_deleted');

CREATE TABLE raw_observations (
    observation_id TEXT PRIMARY KEY NOT NULL,
    projected_session_id TEXT NOT NULL,
    harness TEXT NOT NULL CHECK (harness IN ('claude_code', 'codex', 'opencode')),
    observed_at TEXT NOT NULL,
    observed_at_epoch INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    channel TEXT NOT NULL,
    source_priority INTEGER NOT NULL CHECK (source_priority BETWEEN 1 AND 10),
    captured_at TEXT NOT NULL,
    captured_at_epoch INTEGER NOT NULL,
    native_event_id TEXT,
    native_resume_id TEXT,
    native_session_key TEXT NOT NULL,
    native_sequence INTEGER,
    parser_version TEXT NOT NULL,
    confidence TEXT NOT NULL,
    coverage_json TEXT NOT NULL CHECK (json_valid(coverage_json)),
    inserted_at_epoch INTEGER NOT NULL
) STRICT;

CREATE INDEX raw_observations_session_idx
    ON raw_observations(projected_session_id, observed_at_epoch);
CREATE INDEX raw_observations_capture_idx
    ON raw_observations(captured_at_epoch);

CREATE TABLE projects (
    project_id TEXT PRIMARY KEY NOT NULL,
    native_project_id TEXT,
    name TEXT,
    path TEXT,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL
) STRICT;

CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY NOT NULL,
    harness TEXT NOT NULL,
    account_id TEXT,
    project_id TEXT REFERENCES projects(project_id) ON DELETE SET NULL,
    native_session_key TEXT NOT NULL,
    native_resume_id TEXT,
    branch TEXT,
    title TEXT,
    started_at TEXT NOT NULL,
    started_at_epoch INTEGER NOT NULL,
    ended_at TEXT,
    ended_at_epoch INTEGER,
    state TEXT NOT NULL CHECK (state IN ('active', 'completed', 'interrupted', 'unknown')),
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL
) STRICT;

CREATE INDEX sessions_project_idx ON sessions(project_id, started_at_epoch DESC);
CREATE INDEX sessions_harness_idx ON sessions(harness, started_at_epoch DESC);
CREATE INDEX sessions_branch_idx ON sessions(branch, started_at_epoch DESC);

CREATE TABLE messages (
    message_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
    turn_id TEXT,
    native_message_id TEXT,
    role TEXT NOT NULL,
    text TEXT,
    created_at TEXT NOT NULL,
    created_at_epoch INTEGER NOT NULL,
    winning_observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    source_priority INTEGER NOT NULL,
    captured_at_epoch INTEGER NOT NULL,
    conflict INTEGER NOT NULL DEFAULT 0 CHECK (conflict IN (0, 1))
) STRICT;

CREATE INDEX messages_session_idx ON messages(session_id, created_at_epoch);

CREATE TABLE projection_sources (
    projection_kind TEXT NOT NULL,
    projection_key TEXT NOT NULL,
    observation_id TEXT NOT NULL REFERENCES raw_observations(observation_id) ON DELETE CASCADE,
    selected INTEGER NOT NULL CHECK (selected IN (0, 1)),
    source_priority INTEGER NOT NULL,
    conflict INTEGER NOT NULL DEFAULT 0 CHECK (conflict IN (0, 1)),
    PRIMARY KEY (projection_kind, projection_key, observation_id)
) STRICT;

CREATE INDEX projection_sources_observation_idx
    ON projection_sources(observation_id, projection_kind, projection_key);

CREATE VIRTUAL TABLE message_fts USING fts5(
    message_id UNINDEXED,
    session_id UNINDEXED,
    text,
    project,
    branch,
    harness UNINDEXED,
    tool,
    skill,
    agent,
    tokenize = 'unicode61 remove_diacritics 2'
);
