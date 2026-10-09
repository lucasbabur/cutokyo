-- Native-history import bookkeeping. These rows are cursors and deletion memory,
-- not evidence: raw observations remain the only immutable history.
CREATE TABLE history_import_sources (
    source_key TEXT PRIMARY KEY NOT NULL,
    harness TEXT NOT NULL,
    native_session_key TEXT,
    format TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'imported', 'in_progress', 'unsupported', 'failed',
        'skipped_deleted', 'skipped_subagent', 'empty'
    )),
    size_bytes INTEGER NOT NULL,
    stamp INTEGER NOT NULL,
    byte_offset INTEGER NOT NULL,
    tail_hash TEXT,
    observations INTEGER NOT NULL DEFAULT 0,
    malformed_lines INTEGER NOT NULL DEFAULT 0,
    coverage_state TEXT NOT NULL,
    note TEXT,
    meta_json TEXT NOT NULL DEFAULT '{}',
    updated_at_epoch INTEGER NOT NULL
) STRICT;
CREATE INDEX history_import_sources_harness_idx ON history_import_sources(harness, status);

-- A deleted session must not return silently when its native file still exists.
-- Only an explicit restore-deleted import clears these rows.
CREATE TABLE history_import_tombstones (
    harness TEXT NOT NULL,
    native_session_key TEXT NOT NULL,
    deleted_at_epoch INTEGER NOT NULL,
    PRIMARY KEY (harness, native_session_key)
) STRICT;

-- Import merges live and historical evidence by native message identity.
CREATE INDEX messages_native_idx ON messages(session_id, native_message_id);

-- Bulk import rewrites each touched session's search document once per batch instead
-- of once per inserted row. The import transaction sets this marker, rebuilds the
-- affected documents, then removes the marker before it commits, so ordinary writers
-- and readers never observe it.
DROP TRIGGER session_search_insert;
CREATE TRIGGER session_search_insert AFTER INSERT ON sessions
WHEN NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress') BEGIN
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=new.session_id;
END;
DROP TRIGGER session_search_update;
CREATE TRIGGER session_search_update AFTER UPDATE ON sessions
WHEN NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress') BEGIN
    DELETE FROM session_search WHERE session_id=old.session_id;
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=new.session_id;
END;
DROP TRIGGER session_search_project;
CREATE TRIGGER session_search_project AFTER UPDATE ON projects
WHEN NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress') BEGIN
    DELETE FROM session_search WHERE session_id IN (SELECT session_id FROM sessions WHERE project_id=new.project_id);
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id IN (SELECT session_id FROM sessions WHERE project_id=new.project_id);
END;
DROP TRIGGER session_search_message_insert;
CREATE TRIGGER session_search_message_insert AFTER INSERT ON messages BEGIN
    INSERT INTO canonical_message_search VALUES (new.session_id, new.message_id, new.text);
    DELETE FROM session_search WHERE session_id=new.session_id
        AND NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress');
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=new.session_id
        AND NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress');
END;
DROP TRIGGER session_search_message_update;
CREATE TRIGGER session_search_message_update AFTER UPDATE ON messages BEGIN
    DELETE FROM canonical_message_search WHERE message_id=old.message_id;
    INSERT INTO canonical_message_search VALUES (new.session_id, new.message_id, new.text);
    DELETE FROM session_search WHERE session_id IN (old.session_id, new.session_id)
        AND NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress');
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id IN (old.session_id, new.session_id)
        AND NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress');
END;
