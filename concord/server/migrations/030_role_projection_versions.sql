ALTER TABLE servers
ADD COLUMN role_projection_version INTEGER NOT NULL DEFAULT 0
CHECK(role_projection_version >= 0);
