-- Migration 016: Add BEFORE DELETE trigger for FTS sync
-- The existing triggers handle INSERT, UPDATE of content, and soft-delete (deleted_at).
-- This trigger handles actual row deletion from the messages table, which can happen
-- after VACUUM or manual deletion, preventing FTS rowid desync.

CREATE TRIGGER IF NOT EXISTS messages_fts_hard_delete BEFORE DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.rowid, old.content);
END;
