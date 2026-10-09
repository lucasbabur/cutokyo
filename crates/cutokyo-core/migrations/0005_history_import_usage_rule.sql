-- History-import parsers v1 counted Claude, Codex and OpenCode input tokens by three
-- different rules. Their evidence is purged and re-imported under parser v2 by the
-- projection rebuild that DERIVE_VERSION triggers (the harnesses' files are read-only
-- and still on disk). The purge cannot run here: deleting raw rows cascades into every
-- child table, none of which index their observation column, so it would be quadratic.

-- A projection rebuild replays every observation. The per-row search triggers would
-- rewrite a session's whole document, or scan an unindexed column, once per message.
-- The rebuild sets this marker, clears both search tables, replays, and repopulates them
-- once. Outside a rebuild the triggers behave exactly as before.
DROP TRIGGER session_search_delete;
CREATE TRIGGER session_search_delete AFTER DELETE ON sessions
WHEN NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_rebuild_in_progress') BEGIN
    DELETE FROM session_search WHERE session_id=old.session_id;
END;
DROP TRIGGER session_search_message_delete;
CREATE TRIGGER session_search_message_delete AFTER DELETE ON messages
WHEN NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_rebuild_in_progress') BEGIN
    DELETE FROM canonical_message_search WHERE message_id=old.message_id;
    DELETE FROM session_search WHERE session_id=old.session_id;
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=old.session_id;
END;
DROP TRIGGER session_search_message_insert;
CREATE TRIGGER session_search_message_insert AFTER INSERT ON messages
WHEN NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_rebuild_in_progress') BEGIN
    INSERT INTO canonical_message_search VALUES (new.session_id, new.message_id, new.text);
    DELETE FROM session_search WHERE session_id=new.session_id
        AND NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress');
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=new.session_id
        AND NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress');
END;
DROP TRIGGER session_search_message_update;
CREATE TRIGGER session_search_message_update AFTER UPDATE ON messages
WHEN NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_rebuild_in_progress') BEGIN
    DELETE FROM canonical_message_search WHERE message_id=old.message_id;
    INSERT INTO canonical_message_search VALUES (new.session_id, new.message_id, new.text);
    DELETE FROM session_search WHERE session_id IN (old.session_id, new.session_id)
        AND NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress');
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id IN (old.session_id, new.session_id)
        AND NOT EXISTS (SELECT 1 FROM schema_meta WHERE key='bulk_import_in_progress');
END;
