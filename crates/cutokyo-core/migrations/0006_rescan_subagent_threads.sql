-- Codex subagent threads were skipped with their cursor at end of file. Their usage is
-- now read, so forget those cursors and the next import scans them from the start.
DELETE FROM history_import_sources
WHERE harness = 'codex' AND note = 'subagent thread';
