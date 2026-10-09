-- Search the selected session metadata and transcript, one FTS document per session.
-- This is derived data. Triggers keep mutations and index updates in one transaction.
CREATE VIRTUAL TABLE session_search USING fts5(
    session_id UNINDEXED,
    title,
    project,
    branch,
    native_id,
    transcript,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE VIEW session_search_documents AS
SELECT s.session_id, s.title,
       trim(COALESCE(p.name, '') || ' ' || COALESCE(p.path, '') || ' ' || COALESCE(s.project_id, '')) AS project,
       s.branch,
       s.native_session_key || ' ' || COALESCE(s.native_resume_id, '') || ' ' || s.session_id AS native_id,
       (SELECT group_concat(text, char(10)) FROM
           (SELECT text FROM messages WHERE session_id=s.session_id ORDER BY created_at_epoch, message_id)) AS transcript
FROM sessions s LEFT JOIN projects p ON p.project_id=s.project_id;

INSERT INTO session_search SELECT * FROM session_search_documents;

-- A phrase must occur inside one canonical message, including retained text after
-- a metadata-only winning observation. Raw observation FTS cannot establish this.
CREATE VIRTUAL TABLE canonical_message_search USING fts5(
    session_id UNINDEXED,
    message_id UNINDEXED,
    text,
    tokenize = 'unicode61 remove_diacritics 2'
);
INSERT INTO canonical_message_search SELECT session_id, message_id, text FROM messages;

CREATE TRIGGER session_search_insert AFTER INSERT ON sessions BEGIN
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=new.session_id;
END;
CREATE TRIGGER session_search_update AFTER UPDATE ON sessions BEGIN
    DELETE FROM session_search WHERE session_id=old.session_id;
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=new.session_id;
END;
CREATE TRIGGER session_search_delete AFTER DELETE ON sessions BEGIN
    DELETE FROM session_search WHERE session_id=old.session_id;
END;
CREATE TRIGGER session_search_project AFTER UPDATE ON projects BEGIN
    DELETE FROM session_search WHERE session_id IN (SELECT session_id FROM sessions WHERE project_id=new.project_id);
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id IN (SELECT session_id FROM sessions WHERE project_id=new.project_id);
END;
CREATE TRIGGER session_search_message_insert AFTER INSERT ON messages BEGIN
    INSERT INTO canonical_message_search VALUES (new.session_id, new.message_id, new.text);
    DELETE FROM session_search WHERE session_id=new.session_id;
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=new.session_id;
END;
CREATE TRIGGER session_search_message_update AFTER UPDATE ON messages BEGIN
    DELETE FROM canonical_message_search WHERE message_id=old.message_id;
    INSERT INTO canonical_message_search VALUES (new.session_id, new.message_id, new.text);
    DELETE FROM session_search WHERE session_id IN (old.session_id, new.session_id);
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id IN (old.session_id, new.session_id);
END;
CREATE TRIGGER session_search_message_delete AFTER DELETE ON messages BEGIN
    DELETE FROM canonical_message_search WHERE message_id=old.message_id;
    DELETE FROM session_search WHERE session_id=old.session_id;
    INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=old.session_id;
END;
