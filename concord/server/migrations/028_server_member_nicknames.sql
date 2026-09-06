-- Durable per-server display names were exposed by the application before the
-- membership table had a column capable of storing them.
ALTER TABLE server_members ADD COLUMN nickname TEXT;
