-- Message timestamps remain in their historical storage representation.  Use
-- SQLite's time normalization for chronological search without rewriting the
-- payload exposed through history and replay.
CREATE INDEX IF NOT EXISTS idx_messages_channel_chronology
ON messages(channel_id, julianday(created_at) DESC, id DESC)
WHERE deleted_at IS NULL;
